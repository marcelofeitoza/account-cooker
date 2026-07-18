# Jupiter planning fixtures

These JSON files model the documented Jupiter v1 quote and `swap-instructions`
response shapes. They are deterministic, unsigned planning inputs used for HTTP,
deserialization, and rejection tests only.

They do not constitute swap acceptance. A successful swap is accepted only when the
locally built v0 transaction executes through verified Surfpool and its confirmed
metadata proves both input and output token deltas.
