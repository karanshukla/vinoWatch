// Runs vino-npu-reset.rules against fake polkit requests: only "start" of
// that one unit, from an active local session, gets a yes. polkit itself
// needs root and a seat, so this checks the rule's logic, not polkit.
// Usage: node tests/polkit-rule.js
"use strict";
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const rules = [];
const polkit = {
    Result: { YES: "yes", NO: "no", AUTH_ADMIN: "auth_admin" },
    addRule: (rule) => rules.push(rule),
};
const file = path.join(__dirname, "..", "vino-npu-reset.rules");
vm.runInNewContext(fs.readFileSync(file, "utf8"), { polkit }, { filename: file });
assert.strictEqual(rules.length, 1, "the file adds one rule");

function decide(overrides = {}) {
    const request = {
        id: "org.freedesktop.systemd1.manage-units",
        unit: "vino-npu-reset.service",
        verb: "start",
        local: true,
        active: true,
        ...overrides,
    };
    const details = { unit: request.unit, verb: request.verb };
    const action = { id: request.id, lookup: (key) => details[key] };
    const result = rules[0](action, { local: request.local, active: request.active });
    // polkit treats undefined and null alike: no opinion, next rule.
    return result === undefined ? null : result;
}

const cases = [
    [{}, "yes"],
    [{ verb: "stop" }, null],
    [{ verb: "restart" }, null],
    [{ verb: "reload-or-restart" }, null],
    [{ unit: "sshd.service" }, null],
    [{ unit: "vino-npu-reset.service.d" }, null],
    [{ id: "org.freedesktop.systemd1.manage-unit-files" }, null],
    [{ local: false }, null],
    [{ active: false }, null],
];
for (const [overrides, want] of cases) {
    assert.strictEqual(decide(overrides), want, JSON.stringify(overrides));
}
console.log(`vino-npu-reset.rules: ${cases.length} cases passed`);
