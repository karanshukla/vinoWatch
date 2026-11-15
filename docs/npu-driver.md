# Stuck NPU driver

After heavy NPU use the `intel_vpu` driver can get stuck: any process with two OpenVINO Cores touching the NPU segfaults in `libze_loader`, so vinoPop crashes on every AI action and vinoAuthFace on every scan. Reloading the driver clears it without a reboot.

## Check

With an NPU present the tray menu has **Check NPU driver**. It runs a ~0.3 s probe on demand, never on a timer, and shows the result as a desktop notification as well as in the menu and tooltip. A stuck driver turns the icon amber and keeps it in view.

The probe is vinoPop's: a toy model on the NPU, a second Core, the model again. A segfault means stuck.

It needs a Python that can import openvino, picked in this order:

1. `--probe-python PATH`
2. the venv interpreter of a vino app vinoWatch has seen running
3. `python3`

`vinowatch --check-npu` runs the same probe from a terminal:

| Exit code | Meaning |
|---|---|
| 0 | OK |
| 1 | stuck |
| 3 | couldn't tell |

## Reset

**Reset NPU driver** reloads it (`modprobe -r intel_vpu && modprobe intel_vpu`) through a root oneshot unit, and checks again afterwards. That needs a one-time root step, which `install.sh` doesn't do:

```sh
sudo ~/.local/share/vinowatch/install-npu-reset.sh
sudo ~/.local/share/vinowatch/install-npu-reset.sh --uninstall
```

`install.sh` keeps that script there, with the unit and rule it installs, from the same checked release (or checkout). `sudo ./install-npu-reset.sh` from a checkout or an unpacked release does the same.

It installs `vino-npu-reset.service` and a polkit rule that lets the active local user start that unit, and nothing else, without a password.

`modprobe -r` refuses while anything has `/dev/accel` open, so the item lists the apps holding the NPU and stays disabled until you close them. Nothing is stopped for you. Holders that belong to other users aren't visible; then the reset fails and says so.

Neither the probe nor the reset has run against a stuck driver yet: the polkit rule and the reload were checked on a machine without an NPU.
