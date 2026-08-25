# omarchy-power

Power profiles, fan modes, thermals and charge limits for Linux laptops — one TUI,
one small daemon, no root for the client.

[![CI](https://github.com/MatteoBarzotto/omarchy-power/actions/workflows/ci.yml/badge.svg)](https://github.com/MatteoBarzotto/omarchy-power/actions/workflows/ci.yml)

Built on Omarchy, works on any Arch-based system. Hyprland optional.

```
 omarchy-power  msi-ec  1587EMS1.106
┌ state ──────────────────────────────┐┌ sensors ────────────────────────────┐
│                                     ││                                     │
│   p  Power level    balanced        ││ CPU  66°C  fan 70% ████████ ██      │
│   f  Fan mode       auto            ││                                     │
│   b  Cooler boost   off             ││ GPU  51°C  fan 40% ████████         │
│   s  Battery saver  off             ││                                     │
│ -/+  Charge limit   80%             ││ fan1: 3555 rpm  fan2: 3555 rpm      │
│      Battery        99% (on AC)     ││ fan3: 0 rpm  fan4: 0 rpm            │
│                                     ││                                     │
│                                     ││                                     │
└─────────────────────────────────────┘└─────────────────────────────────────┘
 q quit   r refresh   p/f cycle   b/s toggle   -/+ charge limit
```

## Why this exists

`power-profiles-daemon` is what your desktop actually talks to when you pick a
power profile. Ask it what it drives on a laptop like this one:

```
$ powerprofilesctl list
  power-saver:
    CpuDriver:      intel_pstate
    PlatformDriver: placeholder      <-- nothing is driving your hardware
```

`PlatformDriver: placeholder` means the vendor half of your machine — MSI's
performance shift modes, its fan curves, its battery-saver switch — is never
touched. You pick "power saver", the CPU governor obeys, and the embedded
controller keeps doing whatever it was doing. Half a power profile, and nothing
tells you.

`omarchy-power` supplies the other half. It follows the profile you already
switch from your desktop and applies the matching hardware state, so one switch
means one thing:

| Profile | Performance mode | Fans | Battery saver |
|---|---|---|---|
| `performance` | turbo | aggressive | off |
| `balanced` | comfort | auto | off |
| `power-saver` | eco | silent | on |

Those are defaults, not decisions: the mapping lives in a config file.

## What you get

- A TUI showing thermals, fan RPM, power state and battery, refreshed live.
- Keys to change any of it, without a password prompt when you are at the machine.
- Battery charge limits, so a laptop that lives on mains stops charging at 80%.
- Automatic profile changes: a different profile on battery, if you want one.
- A thermal guard that will not let a quiet fan profile cook the machine.

## Install

On Arch, build the package and let pacman own the files. This builds the
released tarball named in the PKGBUILD, not your checkout:

```
cd packaging
makepkg -si
sudo systemctl enable --now omarchy-powerd.service
omarchy-power
```

From a git checkout, without a package:

```
cargo build --release
sudo packaging/install.sh
omarchy-power
```

Either way you get two binaries, a systemd unit, a D-Bus policy, a polkit policy
and a commented config file. `packaging/uninstall.sh` reverses the second one.

The two do not mix: `install.sh` writes to the same paths, and pacman refuses to
overwrite files it does not own. Run `sudo packaging/uninstall.sh` before your
first `makepkg -si`.

## How it fits together

`omarchy-powerd` runs as root under systemd and is the only thing that ever
writes to sysfs. It owns `org.omarchy.Power1` on the system bus and checks every
write with polkit: an active local session is allowed silently, an ssh session
has to authenticate. The TUI is an ordinary unprivileged client, and so is
anything else you point at the bus:

```
busctl introspect org.omarchy.Power1 /org/omarchy/Power1
busctl call org.omarchy.Power1 /org/omarchy/Power1 org.omarchy.Power1 Snapshot
busctl call org.omarchy.Power1 /org/omarchy/Power1 org.omarchy.Power1 \
    SetPowerLevel s power-saver
```

That is also the fastest way to diagnose a hardware report, which is why the
setters are one-per-attribute instead of a single "apply this profile" call.

The unit is confined: no capabilities at all, `ProtectSystem=strict` with write
access to exactly two sysfs subtrees, no network. `systemd-analyze security`
scores it 1.5.

Without the daemon the TUI still opens, read-only.

## Configuration

`/etc/omarchy-power/config.toml`, every key optional:

```toml
[profiles.power-saver]
power-level = "power-saver"
fan-mode = "silent"
battery-saver = true

# Applied on top of the profile while on battery. Empty by default —
# overriding the profile you just picked, the moment you unplug, is a surprise.
[on-battery]
# power-level = "power-saver"

[thermal]
enabled = true
high-c = 90        # force the fans up here...
low-c = 80         # ...and release only back down here,
cooldown-s = 30    # after it has stayed there this long.
```

Two thermal thresholds rather than one, because a single trip point makes the
fans flap on and off around it.

A typo is an error rather than a silently ignored line: unknown keys and unknown
values are both rejected at load.

## Adding your laptop

Backends never build an absolute path — each one is handed a sysfs root. A
captured directory tree is therefore indistinguishable from real `/sys`, which
means support for your machine can be written and tested by people who do not
own one:

```
omarchy-power dump-fixture my-laptop
tar czf my-laptop.tar.gz my-laptop
```

That captures an explicit list of power-related attributes — no serial numbers,
no identifiers. Skim it anyway, then open an issue with the archive attached.

If you write Rust, `crates/core/src/backend.rs` holds the trait and
`crates/core/src/backends/msi.rs` is a worked example. Point the binaries at a
fixture to develop against hardware you do not have:

```
OMARCHY_POWER_SYSFS=fixtures/msi-katana cargo run -p omarchy-power
```

## Supported hardware

| Vendor | Backend | Covers | Tested on |
|---|---|---|---|
| MSI | `msi-ec` | performance level, fan modes, cooler boost, battery saver, charge limit | Katana, fw 1587EMS1.106 |
| anything with `platform_profile` | `platform-profile` | performance level, charge limit | fixtures only — no confirmed hardware yet |

On MSI, the in-kernel `msi_ec` does not expose these attributes on most models;
the DKMS build from the AUR does.

The `platform-profile` backend needs no vendor driver from us: it drives the
kernel's own `/sys/firmware/acpi/platform_profile` (or the `platform-profile`
class on 6.14 and later) and the standard `charge_control_end_threshold`, which
between them cover ThinkPads, Framework, Dell, ASUS and much of HP. It offers
less than a vendor backend, and honestly so — no standard interface exposes fan
modes, cooler boost or a battery saver, so it reports those as unsupported
instead of pretending. A vendor backend always wins detection where both apply.

Whether it works on your machine is exactly what nobody has checked yet:

```
cat /sys/firmware/acpi/platform_profile_choices
```

If that prints anything, the backend should drive your laptop — an issue saying
it did, or did not, is more useful right now than a feature request.

## Known conflicts

Some systems already have something else writing the same attributes. Omarchy
ships `battery-charge-threshold.service`, which hard-codes a value at every boot,
so whatever you set here is silently replaced on the next restart. TLP does the
same from its own config.

The daemon asks systemd at startup whether any of them are enabled, names them
in the journal, and the TUI prints the name under the charge limit. The bus has
it too:

```
busctl get-property org.omarchy.Power1 /org/omarchy/Power1 \
    org.omarchy.Power1 ChargeThresholdConflicts
```

Whichever one you want to keep, disable the other:

```
sudo systemctl disable --now battery-charge-threshold.service
```

## Status

Early but working end to end on the hardware above. Interfaces may still move
before 1.0.

## License

MIT
