# vinoWatch
Monitors NPU, GPU, CPU, RAM and Energy usage by Intel OpenVino NPU runtimes and then applications that utilise them. 

Run with `cargo run [interval_seconds]`. Detects processes using OpenVINO (by name or loaded `libopenvino*`) and reports CPU/RAM, plus NPU utilisation on Linux via the Intel `ivpu` driver sysfs. A system tray front-end is not yet implemented.
