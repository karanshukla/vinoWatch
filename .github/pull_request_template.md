## Summary

<!-- What changes, and why. -->

## Testing

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] Ran it against a real OpenVINO process, or said below why that wasn't possible

CI has no NPU. Say which of these applies:

- [ ] Verified with an NPU (`ivpu` driver, kernel version below)
- [ ] Verified on a machine without an NPU
- [ ] Logic only; nothing the tray or console prints changed

## Checklist

- [ ] Comments explain *why*, not what, and any measured claim carries a date
- [ ] README / CONTRIBUTING updated if behaviour or setup changed
- [ ] No secrets, tokens or machine-specific paths committed
