# Bounty Eligibility Audit

Checked: 2026-07-18

This audit records the published rules that govern the Account Cooker contribution. It is
separate from the engineering evidence because a local test run cannot establish personal
or platform eligibility.

## Sources

- [Live Privacy-Through-Noise listing](https://superteam.fun/earn/listing/noise)
- [Official Superteam Earn agent protocol](https://superteam.fun/earn/agents/)
- [Superteam Earn terms of use](https://superteam.fun/earn/terms-of-use.pdf)
- [Upstream Account Cooker repository](https://github.com/solanabr/account-cooker)

## Published Facts

At the check time, the listing is open with deadline `2026-07-29T02:59:59.999Z`
(`2026-07-28 23:59:59` BRT), region `Brazil`, type `bounty`, and
`agentAccess: HUMAN_ONLY`. It lists no additional eligibility-question form. Its stated
artifact requirements are:

- Brazilian builders only;
- Rust end to end;
- production-grade, scalable, customizable, realistic, documented tooling;
- open-source MIT code;
- one `account-cooker` winner selected on impact, quality, and contribution volume.

The official agent protocol says only `AGENT_ALLOWED` and `AGENT_ONLY` listings accept
agent submissions. It documents a `403 Agents are not eligible for this listing` response
for an ineligible agent path. Therefore no agent registration or submission API is used for
this contribution.

## AI Assistance

Neither the listing nor the platform terms state that a human entrant may not use AI tools.
`HUMAN_ONLY` is the platform's submission-identity control; it is not published as an
authorship or tool-use prohibition. This contribution is reviewed and submitted by Marcelo
through his human profile, and the draft PR explicitly discloses that development was
AI-assisted under his direction.

On the published rules, disclosed AI-assisted implementation is compatible with a human
submission. A later listing change or direct sponsor instruction would supersede this dated
audit and must be reviewed before submission.

## Rust Interpretation

The product implementation, domain logic, storage, scheduler, adapters, runtime,
evaluation, and CLI are Rust. Shell and YAML files are reproducibility and CI orchestration,
not alternate product implementations. Generated JSON and TOML files are configuration,
fixtures, and evidence. The PR does not claim that these supporting formats are Rust code.

## Human Release Controls

Before submission, Marcelo must personally:

1. confirm the Superteam profile satisfies the Brazil restriction and current account
   requirements;
2. review the PR, disclosure, evidence, and known limitations;
3. submit through the human Superteam interface before the deadline;
4. complete any winner KYC or payout requirements if selected.

Those attestations cannot be delegated to the software or evidence harness. Until Marcelo
approves them, upstream PR 2 remains draft.
