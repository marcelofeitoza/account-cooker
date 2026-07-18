# Account Cooker Evidence Report

Status: **passed** (canonical run at 2026-07-18T21:48:29Z)

## Executed Proofs

- Full CLI lifecycle: read-only doctor and previews, 16-agent fleet generation and local funding, bounded execution, status, idempotent funding replay, and historical confirmation audit with zero recovery submissions.
- Chain acceptance: native SOL, classic SPL with ATA creation, Jupiter exact-input from pinned reviewed state, deterministic/unknown-outcome reconciliation, all six real SIGKILL/restart checkpoints, and native stake lifecycle last.
- Virtual soak: 1000 agents for 30 virtual days across 5 seeds; deterministic replay and all safety proofs passed; peak measured RSS 221462528 bytes.
- Surfpool soak: 1000 real local transactions, one lost response, runtime reconstruction, Surfpool restart, zero duplicate signatures, zero budget violations, and zero unresolved outcomes.

## Measured Result

Full-attacker ROC AUC means were 1.0 for naive uniform, 1.0 for independent weighted, and 0.981407901234568 for persona/session. Persona/session F1 at the fixed 0.55 threshold was 0.7995646990748174. These results show only a measured change in this synthetic benchmark; they do not establish anonymity.

## Interpretation Bound

The common-funder graph remains directly observable. The evaluator is synthetic and does not establish anonymity. Jupiter uses a reviewed pinned state snapshot, native stake is used instead of Marinade, and local Surfpool does not reproduce public-network topology or execution quality. All chain transactions in this pack were signed and executed only against loopback Surfpool. Full local signatures, databases, generated signers, and raw logs are excluded from this pack.
