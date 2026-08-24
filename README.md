# omarchy-power

Hardware power management for Linux laptops — power profiles, charge thresholds,
fan modes and thermals, from one TUI and one small daemon.

Built on Omarchy, works on any Arch-based system with Hyprland. Or without it.

## Why this exists

`power-profiles-daemon` (PPD) is what your desktop actually talks to when you pick
a power profile. On most laptops it only drives the CPU:

```
$ powerprofilesctl list
  power-saver:
    CpuDriver:      intel_pstate
    PlatformDriver: placeholder      <-- nothing is driving your hardware
```

`PlatformDriver: placeholder` means the vendor layer of your machine — MSI's
`shift_mode`, its fan curves, its battery-saver switch — is never touched. You pick
"power saver", the CPU obeys, and the embedded controller keeps running whatever it
was running. You get half a power profile and nothing tells you.

`omarchy-power` fills in that half. It listens for PPD profile changes and applies
the matching hardware state, so one switch means one thing.

## Install

```
cargo build --release
sudo packaging/install.sh
omarchy-power
```

That installs two binaries, a systemd unit, a D-Bus policy and a polkit policy.
`packaging/uninstall.sh` reverses it.

## How it fits together

`omarchy-powerd` runs as root under systemd and is the only thing that writes to
sysfs. It owns `org.omarchy.Power1` on the system bus, and every write is checked
with polkit — an active local session is allowed without a prompt, an ssh session
has to authenticate. The TUI is an ordinary unprivileged client.

Which means the daemon is usable by hand, and that is the fastest way to diagnose
a hardware report:

```
busctl introspect org.omarchy.Power1 /org/omarchy/Power1
busctl call org.omarchy.Power1 /org/omarchy/Power1 org.omarchy.Power1 Snapshot
busctl call org.omarchy.Power1 /org/omarchy/Power1 org.omarchy.Power1 \
    SetPowerLevel s power-saver
```

Without the daemon the TUI still opens, read-only.

## Known conflicts

Some systems already have something else writing the same attributes. Omarchy
ships `battery-charge-threshold.service`, which hard-codes a value at every boot;
whatever you set through omarchy-power is silently replaced on the next restart.
Check before wondering where your setting went:

```
systemctl list-units --all | grep -iE 'batt|charge'
```

Detecting this and saying so out loud is on the list.

## Status

Early — hardware can be read and changed; automatic profile switching is next.

Supported hardware:

| Vendor | Backend | Tested on |
|---|---|---|
| MSI | `msi-ec` | Katana (fw 1587EMS1.106) |

## Adding your laptop

Backends never touch absolute paths — each one is handed a sysfs root, so a backend
can be tested against a captured directory tree instead of real hardware. That means
you can contribute support for your machine without me owning one:

```
omarchy-power dump-fixture my-laptop
tar czf my-laptop.tar.gz my-laptop
```

That captures an explicit list of power-related attributes — no serial numbers, no
identifiers. Skim it anyway, then open an issue with the archive attached. If you write Rust, `crates/core/src/backend.rs`
has the trait and the MSI backend next to it as a worked example.

## License

MIT
