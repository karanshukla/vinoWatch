#!/bin/sh
# Install the root half of vinoWatch's "Reset NPU driver": a oneshot system
# unit that reloads intel_vpu, and a polkit rule that lets the active local
# user start that unit, and nothing else, without a password. install.sh
# stays user-only; run this once, as root, from a checkout or an unpacked
# release:
#   sudo ./install-npu-reset.sh
# --uninstall removes both again.
set -eu

unit=/etc/systemd/system/vino-npu-reset.service
rule=/etc/polkit-1/rules.d/50-vino-npu-reset.rules

if [ "$(id -u)" -ne 0 ]; then
    echo "Run this as root: sudo $0 $*" >&2
    exit 1
fi

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$unit" "$rule"
    systemctl daemon-reload
    echo "Removed the NPU reset unit and its polkit rule."
    exit 0
fi

here=$(cd "$(dirname "$0")" && pwd)
for f in vino-npu-reset.service vino-npu-reset.rules; do
    if [ ! -f "$here/$f" ]; then
        echo "$f isn't next to this script. Run it from a vinoWatch checkout or release." >&2
        exit 1
    fi
done

install -Dm644 "$here/vino-npu-reset.service" "$unit"
install -Dm644 "$here/vino-npu-reset.rules" "$rule"
systemctl daemon-reload
echo "Installed. vinoWatch's tray menu now has \"Reset NPU driver\"."
