# Stuck NPU driver

After heavy NPU workloads, the `intel_vpu` kernel driver can enter a stuck state where any process creating two OpenVINO `Core` instances on the NPU segfaults inside `libze_loader`. When this happens, `vinoPop` crashes on AI actions and `vinoAuthFace` crashes on every scan. Reloading the kernel driver clears the state without rebooting.

## Checking the driver

When an NPU is present, the tray menu includes **Check NPU driver**. It runs a ~0.3 s probe strictly on demand (never on a timer) and displays the result as a desktop notification as well as in the tray menu and tooltip. If the driver is stuck, the tray icon turns amber and stays visible.

The probe uses the same check as `vinoPop`: it compiles and runs a small test model on the NPU, creates a second OpenVINO `Core`, and runs the model again. A segfault indicates that the driver is stuck.

The probe requires a Python interpreter that can import `openvino`, chosen in the following order:

1. `--probe-python PATH`
2. The virtual environment interpreter of a running `vino*` app observed by `vinoWatch`
3. `python3` from `PATH`

You can run the same probe from a terminal with `vinowatch --check-npu`:

| Exit code | Meaning |
|---|---|
| `0` | Driver OK |
| `1` | Driver stuck |
| `3` | Could not determine (e.g., no usable OpenVINO Python environment found) |

## Resetting the driver

**Reset NPU driver** reloads the driver (`modprobe -r intel_vpu && modprobe intel_vpu`) through a root systemd oneshot service, then re-runs the probe to confirm recovery.

Setting up the reset service requires a one-time root command that `install.sh` does not execute automatically:

```sh
sudo ~/.local/share/vinowatch/install-npu-reset.sh
sudo ~/.local/share/vinowatch/install-npu-reset.sh --uninstall
```

`install.sh` places `install-npu-reset.sh`, the service unit, and the polkit rule in `~/.local/share/vinowatch` from the same verified release (or Git checkout). Running `sudo ./install-npu-reset.sh` directly from a checkout or unpacked release does the same thing.

The script installs `vino-npu-reset.service` and a polkit rule allowing the active local user to start only that specific unit without entering a password.

Because `modprobe -r` cannot unload `intel_vpu` while any process has `/dev/accel` open, the menu item lists all visible apps currently using the NPU and stays disabled until you close them. `vinoWatch` never stops processes automatically. If another user's process holds `/dev/accel` open, `vinoWatch` cannot inspect it and the reset reports the failure.

*Note: Neither the probe nor the reset has been run against a live stuck NPU driver yet; the polkit rule and module reload workflow were verified on a machine without an NPU.*
