# Fixtures

Captured `sysfs` trees from real laptops. Each directory mirrors the layout of
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
