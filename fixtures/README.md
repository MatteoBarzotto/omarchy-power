# Fixtures

`sysfs` trees, mostly captured from real laptops. Each directory mirrors the layout of
`/sys`, holding only the attributes the backends read.

They serve two purposes: unit tests run against them, and the binaries can be
pointed at one directly, which is how hardware nobody on the project owns still
gets exercised:

```
OMARCHY_POWER_SYSFS=fixtures/msi-katana cargo run -p omarchy-power
```

Contributing one: run `omarchy-power dump-fixture`, check the result for anything
you would rather not publish (serial numbers are excluded, but skim it anyway),
and attach it to an issue.

| Directory | Machine | Driver |
|---|---|---|
| `msi-katana` | MSI Katana, fw 1587EMS1.106 | `msi-ec` (DKMS) |
| `platform-profile-acpi` | none — built by hand | kernel `platform_profile`, the original file |
| `platform-profile-class` | none — built by hand | kernel `platform_profile`, the class added in 6.14 |

The last two were written from the kernel's documentation rather than captured,
because the backend they exercise is driven by interfaces the kernel defines and
nobody on the project owns a machine that has them. That makes them a weaker
kind of evidence than the rest: they prove the code does what the documentation
says, not that the documentation matches your laptop. A real capture from a
ThinkPad or a Framework would replace them, and is the single most useful thing
anyone with such a machine can contribute.
