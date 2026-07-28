# Account Cooker Evaluation

## Experiment

- Controllers: 10
- Agents per controller: 10
- Days: 30
- Events per agent/day: 2
- Held-out seeds: [11, 23, 37, 51, 71]
- Fixed threshold: 0.550
- Funding schemes: [DedicatedPerOperator, PooledMixedRounds, PooledPerOperatorRounds]
- Shared disbursers: 8
- Round length (hours): 24
- Top-ups per account: 4
- Uniform denomination (lamports): 600000000

## Funding Provenance

Every scheme runs the same behavior, budgets, attacker, threshold, and participation schedule. Only the payer assignment changes, so any movement in these columns is attributable to the payer assignment and nothing else. Three funding attacks are reported. Funding is the common-funder overlap. Funding round is the same overlap after it also reads the batching round. Funding batch is the scheme-aware attack: it assumes the adversary knows accounts are dealt into pooled batches and weights a shared payer-and-round batch by how few accounts were in it, so a shared batch of two counts far more than a shared batch of twenty. All three read only what the chain publishes.

| Funding scheme | Planner | Funding AUC mean [95% CI] | Funding round AUC mean | Funding batch AUC mean | Funding separation mean | Composite AUC mean |
|---|---|---:|---:|---:|---:|---:|
| DedicatedPerOperator | NaiveUniform | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 | 0.9814 |
| PooledMixedRounds | NaiveUniform | 0.5054 [0.4932, 0.5177] | 0.4977 | 0.4980 | 0.0028 | 1.0000 |
| PooledMixedRounds | IndependentWeighted | 0.5054 [0.4932, 0.5177] | 0.4977 | 0.4980 | 0.0028 | 0.7111 |
| PooledMixedRounds | PersonaSession | 0.5054 [0.4932, 0.5177] | 0.4977 | 0.4980 | 0.0028 | 0.6751 |
| PooledPerOperatorRounds | NaiveUniform | 0.7203 [0.7045, 0.7361] | 0.9487 | 0.9474 | 0.1607 | 1.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 0.7203 [0.7045, 0.7361] | 0.9487 | 0.9474 | 0.1607 | 0.8378 |
| PooledPerOperatorRounds | PersonaSession | 0.7203 [0.7045, 0.7361] | 0.9487 | 0.9474 | 0.1607 | 0.7938 |

The fixed composite holds the funding-round and funding-batch weights at zero so composite numbers stay comparable with the pre-mitigation baseline. That choice is conservative for the pooled arm: both extra families sit near chance there, so folding them into the composite would dilute the informative families and push the pooled composite lower than reported.

### Funding Schedule Cost

Transfer counts are equal across schemes by construction, because every scheme replays one shared participation schedule. The cost of pooling is therefore read against operator practice rather than against another row here: a naive operator provisions each account once from one wallet, so the pooled schedule multiplies funding transfers by the top-ups-per-account setting and adds the operator's own deposits into the pool, which this evaluator does not model. The smallest-batch column is the residual: a round and payer batch with one recipient gives that transfer no cover at all.

| Funding scheme | Seed | Transfers | Distinct payers | Rounds | Mean batch recipients | Smallest batch | Mean observed payers per account |
|---|---:|---:|---:|---:|---:|---:|---:|
| DedicatedPerOperator | 11 | 400 | 10 | 30 | 2.08 | 1 | 1.00 |
| DedicatedPerOperator | 23 | 400 | 10 | 30 | 2.06 | 1 | 1.00 |
| DedicatedPerOperator | 37 | 400 | 10 | 30 | 1.99 | 1 | 1.00 |
| DedicatedPerOperator | 51 | 400 | 10 | 30 | 2.04 | 1 | 1.00 |
| DedicatedPerOperator | 71 | 400 | 10 | 30 | 1.96 | 1 | 1.00 |
| PooledMixedRounds | 11 | 400 | 8 | 30 | 1.72 | 1 | 3.39 |
| PooledMixedRounds | 23 | 400 | 8 | 30 | 1.72 | 1 | 3.34 |
| PooledMixedRounds | 37 | 400 | 8 | 30 | 1.75 | 1 | 3.29 |
| PooledMixedRounds | 51 | 400 | 8 | 30 | 1.73 | 1 | 3.37 |
| PooledMixedRounds | 71 | 400 | 8 | 30 | 1.77 | 1 | 3.36 |
| PooledPerOperatorRounds | 11 | 400 | 8 | 30 | 2.84 | 1 | 3.35 |
| PooledPerOperatorRounds | 23 | 400 | 8 | 30 | 2.99 | 1 | 3.25 |
| PooledPerOperatorRounds | 37 | 400 | 8 | 30 | 2.90 | 1 | 3.28 |
| PooledPerOperatorRounds | 51 | 400 | 8 | 30 | 2.90 | 1 | 3.42 |
| PooledPerOperatorRounds | 71 | 400 | 8 | 30 | 2.78 | 1 | 3.24 |

## Composite Results

| Funding scheme | Planner | Ablation | ROC AUC mean [95% CI] | F1 mean [95% CI] | Precision@K mean | ARI mean | NMI mean |
|---|---|---|---:|---:|---:|---:|---:|
| DedicatedPerOperator | NaiveUniform | none | 1.0000 [1.0000, 1.0000] | 0.8257 [0.8257, 0.8257] | 1.0000 | 0.7179 | 0.9052 |
| DedicatedPerOperator | NaiveUniform | without_timing | 1.0000 [1.0000, 1.0000] | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | without_amount | 1.0000 [1.0000, 1.0000] | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | without_sequence | 1.0000 [1.0000, 1.0000] | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | without_synchrony | 1.0000 [1.0000, 1.0000] | 0.6000 [0.6000, 0.6000] | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | without_funding | 1.0000 [1.0000, 1.0000] | 0.6000 [0.6000, 0.6000] | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | IndependentWeighted | none | 1.0000 [1.0000, 1.0000] | 0.1993 [0.1946, 0.2040] | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | without_timing | 1.0000 [1.0000, 1.0000] | 0.6186 [0.6037, 0.6334] | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | without_amount | 1.0000 [1.0000, 1.0000] | 0.3823 [0.3683, 0.3964] | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | without_sequence | 1.0000 [1.0000, 1.0000] | 0.6952 [0.6740, 0.7164] | 1.0000 | 0.0087 | 0.0751 |
| DedicatedPerOperator | IndependentWeighted | without_synchrony | 1.0000 [1.0000, 1.0000] | 0.1667 [0.1667, 0.1667] | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | without_funding | 0.8035 [0.7944, 0.8126] | 0.1667 [0.1667, 0.1667] | 0.3618 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | none | 0.9814 [0.9808, 0.9820] | 0.7996 [0.7916, 0.8075] | 0.7791 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | without_timing | 0.9982 [0.9976, 0.9988] | 0.9185 [0.9080, 0.9289] | 0.9329 | 0.1231 | 0.4888 |
| DedicatedPerOperator | PersonaSession | without_amount | 0.9817 [0.9810, 0.9825] | 0.8229 [0.8166, 0.8291] | 0.7796 | 0.0000 | 0.0131 |
| DedicatedPerOperator | PersonaSession | without_sequence | 0.9831 [0.9827, 0.9836] | 0.8461 [0.8424, 0.8499] | 0.8093 | 0.0089 | 0.1261 |
| DedicatedPerOperator | PersonaSession | without_synchrony | 0.9963 [0.9957, 0.9970] | 0.6099 [0.6006, 0.6192] | 0.8956 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | without_funding | 0.7530 [0.7466, 0.7595] | 0.2525 [0.2461, 0.2589] | 0.1413 | 0.0000 | 0.0000 |
| PooledMixedRounds | NaiveUniform | none | 1.0000 [1.0000, 1.0000] | 0.6538 [0.6486, 0.6590] | 1.0000 | 0.2648 | 0.6444 |
| PooledMixedRounds | NaiveUniform | without_timing | 1.0000 [1.0000, 1.0000] | 0.9551 [0.9466, 0.9636] | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | without_amount | 1.0000 [1.0000, 1.0000] | 0.8033 [0.7974, 0.8093] | 1.0000 | 0.4568 | 0.7703 |
| PooledMixedRounds | NaiveUniform | without_sequence | 1.0000 [1.0000, 1.0000] | 0.7761 [0.7680, 0.7842] | 1.0000 | 0.0435 | 0.3757 |
| PooledMixedRounds | NaiveUniform | without_synchrony | 1.0000 [1.0000, 1.0000] | 0.5220 [0.5172, 0.5267] | 0.9947 | 0.0000 | 0.0000 |
| PooledMixedRounds | NaiveUniform | without_funding | 1.0000 [1.0000, 1.0000] | 0.6000 [0.6000, 0.6000] | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | IndependentWeighted | none | 0.7111 [0.7059, 0.7163] | 0.1709 [0.1699, 0.1718] | 0.2049 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | without_timing | 0.7175 [0.7096, 0.7253] | 0.2148 [0.2107, 0.2190] | 0.2111 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | without_amount | 0.7113 [0.7058, 0.7168] | 0.1931 [0.1895, 0.1967] | 0.2071 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | without_sequence | 0.7187 [0.7133, 0.7241] | 0.2199 [0.2163, 0.2236] | 0.2107 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | without_synchrony | 0.7111 [0.7060, 0.7163] | 0.1667 [0.1667, 0.1667] | 0.2049 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | without_funding | 0.8035 [0.7944, 0.8126] | 0.1667 [0.1667, 0.1667] | 0.3618 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | none | 0.6751 [0.6637, 0.6865] | 0.1411 [0.1278, 0.1545] | 0.1369 | 0.0000 | 0.0131 |
| PooledMixedRounds | PersonaSession | without_timing | 0.6977 [0.6863, 0.7090] | 0.2505 [0.2463, 0.2547] | 0.1880 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | without_amount | 0.6775 [0.6657, 0.6893] | 0.0945 [0.0863, 0.1027] | 0.1373 | 0.0010 | 0.1376 |
| PooledMixedRounds | PersonaSession | without_sequence | 0.6873 [0.6751, 0.6995] | 0.0852 [0.0778, 0.0927] | 0.1378 | 0.0031 | 0.2053 |
| PooledMixedRounds | PersonaSession | without_synchrony | 0.6771 [0.6656, 0.6887] | 0.2229 [0.2151, 0.2308] | 0.1422 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | without_funding | 0.7530 [0.7466, 0.7595] | 0.2525 [0.2461, 0.2589] | 0.1413 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | NaiveUniform | none | 1.0000 [1.0000, 1.0000] | 0.6472 [0.6336, 0.6608] | 1.0000 | 0.3176 | 0.6754 |
| PooledPerOperatorRounds | NaiveUniform | without_timing | 1.0000 [1.0000, 1.0000] | 0.9527 [0.9346, 0.9709] | 1.0000 | 0.5519 | 0.8295 |
| PooledPerOperatorRounds | NaiveUniform | without_amount | 1.0000 [1.0000, 1.0000] | 0.7996 [0.7800, 0.8192] | 1.0000 | 0.4568 | 0.7703 |
| PooledPerOperatorRounds | NaiveUniform | without_sequence | 1.0000 [1.0000, 1.0000] | 0.7805 [0.7719, 0.7892] | 1.0000 | 0.0435 | 0.3757 |
| PooledPerOperatorRounds | NaiveUniform | without_synchrony | 1.0000 [1.0000, 1.0000] | 0.5252 [0.5183, 0.5321] | 0.9996 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | NaiveUniform | without_funding | 1.0000 [1.0000, 1.0000] | 0.6000 [0.6000, 0.6000] | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | IndependentWeighted | none | 0.8378 [0.8245, 0.8512] | 0.1712 [0.1702, 0.1722] | 0.3893 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | without_timing | 0.8467 [0.8341, 0.8594] | 0.2287 [0.2256, 0.2318] | 0.3960 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | without_amount | 0.8399 [0.8269, 0.8529] | 0.1991 [0.1960, 0.2023] | 0.3902 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | without_sequence | 0.8495 [0.8364, 0.8627] | 0.2391 [0.2365, 0.2416] | 0.3924 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | without_synchrony | 0.8381 [0.8248, 0.8514] | 0.1667 [0.1667, 0.1667] | 0.3924 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | without_funding | 0.8035 [0.7944, 0.8126] | 0.1667 [0.1667, 0.1667] | 0.3618 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | none | 0.7938 [0.7837, 0.8039] | 0.2512 [0.2412, 0.2612] | 0.2280 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | without_timing | 0.8277 [0.8157, 0.8396] | 0.3661 [0.3469, 0.3852] | 0.3324 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | without_amount | 0.7961 [0.7852, 0.8071] | 0.1582 [0.1429, 0.1735] | 0.2258 | -0.0001 | 0.0454 |
| PooledPerOperatorRounds | PersonaSession | without_sequence | 0.8080 [0.7998, 0.8161] | 0.1272 [0.1151, 0.1393] | 0.2360 | 0.0004 | 0.1032 |
| PooledPerOperatorRounds | PersonaSession | without_synchrony | 0.7981 [0.7884, 0.8078] | 0.2520 [0.2495, 0.2545] | 0.2373 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | without_funding | 0.7530 [0.7466, 0.7595] | 0.2525 [0.2461, 0.2589] | 0.1413 | 0.0000 | 0.0000 |

## Per-Seed Results

| Funding scheme | Planner | Seed | Ablation | Trace hash | Events | Pairs | ROC AUC | F1 | Precision@K | ARI | NMI |
|---|---|---:|---|---|---:|---:|---:|---:|---:|---:|---:|
| DedicatedPerOperator | NaiveUniform | 11 | none | `32d024da4d9857829e2943a2538fb4dcbc456c77e81166fdb6c102325f931332` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| DedicatedPerOperator | NaiveUniform | 11 | without_timing | `32d024da4d9857829e2943a2538fb4dcbc456c77e81166fdb6c102325f931332` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | without_amount | `32d024da4d9857829e2943a2538fb4dcbc456c77e81166fdb6c102325f931332` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | without_sequence | `32d024da4d9857829e2943a2538fb4dcbc456c77e81166fdb6c102325f931332` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | without_synchrony | `32d024da4d9857829e2943a2538fb4dcbc456c77e81166fdb6c102325f931332` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 11 | without_funding | `32d024da4d9857829e2943a2538fb4dcbc456c77e81166fdb6c102325f931332` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 23 | none | `bfb3a3130294783a1751d94fad9aedd8f700bd9023456e8024c78f1575241cb0` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| DedicatedPerOperator | NaiveUniform | 23 | without_timing | `bfb3a3130294783a1751d94fad9aedd8f700bd9023456e8024c78f1575241cb0` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | without_amount | `bfb3a3130294783a1751d94fad9aedd8f700bd9023456e8024c78f1575241cb0` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | without_sequence | `bfb3a3130294783a1751d94fad9aedd8f700bd9023456e8024c78f1575241cb0` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | without_synchrony | `bfb3a3130294783a1751d94fad9aedd8f700bd9023456e8024c78f1575241cb0` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 23 | without_funding | `bfb3a3130294783a1751d94fad9aedd8f700bd9023456e8024c78f1575241cb0` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 37 | none | `d05f47eab5e39daebc9785eab95065cb6d3650cf97b2c77dbe200b93bda62a0e` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| DedicatedPerOperator | NaiveUniform | 37 | without_timing | `d05f47eab5e39daebc9785eab95065cb6d3650cf97b2c77dbe200b93bda62a0e` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | without_amount | `d05f47eab5e39daebc9785eab95065cb6d3650cf97b2c77dbe200b93bda62a0e` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | without_sequence | `d05f47eab5e39daebc9785eab95065cb6d3650cf97b2c77dbe200b93bda62a0e` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | without_synchrony | `d05f47eab5e39daebc9785eab95065cb6d3650cf97b2c77dbe200b93bda62a0e` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 37 | without_funding | `d05f47eab5e39daebc9785eab95065cb6d3650cf97b2c77dbe200b93bda62a0e` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 51 | none | `da3cdf61fc040b06c26e7d5fd122405313496fcd41cd7a0d134e9791813337b9` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| DedicatedPerOperator | NaiveUniform | 51 | without_timing | `da3cdf61fc040b06c26e7d5fd122405313496fcd41cd7a0d134e9791813337b9` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | without_amount | `da3cdf61fc040b06c26e7d5fd122405313496fcd41cd7a0d134e9791813337b9` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | without_sequence | `da3cdf61fc040b06c26e7d5fd122405313496fcd41cd7a0d134e9791813337b9` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | without_synchrony | `da3cdf61fc040b06c26e7d5fd122405313496fcd41cd7a0d134e9791813337b9` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 51 | without_funding | `da3cdf61fc040b06c26e7d5fd122405313496fcd41cd7a0d134e9791813337b9` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 71 | none | `e3e23bdaa0c44c7487acb924fd4d7a8ac29587604c6aeac8725fa30dae949189` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| DedicatedPerOperator | NaiveUniform | 71 | without_timing | `e3e23bdaa0c44c7487acb924fd4d7a8ac29587604c6aeac8725fa30dae949189` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | without_amount | `e3e23bdaa0c44c7487acb924fd4d7a8ac29587604c6aeac8725fa30dae949189` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | without_sequence | `e3e23bdaa0c44c7487acb924fd4d7a8ac29587604c6aeac8725fa30dae949189` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | without_synchrony | `e3e23bdaa0c44c7487acb924fd4d7a8ac29587604c6aeac8725fa30dae949189` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | NaiveUniform | 71 | without_funding | `e3e23bdaa0c44c7487acb924fd4d7a8ac29587604c6aeac8725fa30dae949189` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| DedicatedPerOperator | IndependentWeighted | 11 | none | `3450b2517869c68603fbabae274fa22162b976acc6c8b5bc8897134abf19381c` | 6000 | 4950 | 1.0000 | 0.1933 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 11 | without_timing | `3450b2517869c68603fbabae274fa22162b976acc6c8b5bc8897134abf19381c` | 6000 | 4950 | 1.0000 | 0.6069 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 11 | without_amount | `3450b2517869c68603fbabae274fa22162b976acc6c8b5bc8897134abf19381c` | 6000 | 4950 | 1.0000 | 0.3663 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 11 | without_sequence | `3450b2517869c68603fbabae274fa22162b976acc6c8b5bc8897134abf19381c` | 6000 | 4950 | 1.0000 | 0.6844 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 11 | without_synchrony | `3450b2517869c68603fbabae274fa22162b976acc6c8b5bc8897134abf19381c` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 11 | without_funding | `3450b2517869c68603fbabae274fa22162b976acc6c8b5bc8897134abf19381c` | 6000 | 4950 | 0.7998 | 0.1667 | 0.3622 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | none | `4f6967975b1f395808fb2a51d1a5604a16bcfe6b20ea3d9161141d317f2f9ed9` | 6000 | 4950 | 1.0000 | 0.1995 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | without_timing | `4f6967975b1f395808fb2a51d1a5604a16bcfe6b20ea3d9161141d317f2f9ed9` | 6000 | 4950 | 1.0000 | 0.6048 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | without_amount | `4f6967975b1f395808fb2a51d1a5604a16bcfe6b20ea3d9161141d317f2f9ed9` | 6000 | 4950 | 1.0000 | 0.3707 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | without_sequence | `4f6967975b1f395808fb2a51d1a5604a16bcfe6b20ea3d9161141d317f2f9ed9` | 6000 | 4950 | 1.0000 | 0.6813 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | without_synchrony | `4f6967975b1f395808fb2a51d1a5604a16bcfe6b20ea3d9161141d317f2f9ed9` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | without_funding | `4f6967975b1f395808fb2a51d1a5604a16bcfe6b20ea3d9161141d317f2f9ed9` | 6000 | 4950 | 0.7957 | 0.1667 | 0.3511 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | none | `719a9a9b98a4fad2bc7ac88232b65d60c28072a54bbbfb5cdcba556472deaa17` | 6000 | 4950 | 1.0000 | 0.2059 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | without_timing | `719a9a9b98a4fad2bc7ac88232b65d60c28072a54bbbfb5cdcba556472deaa17` | 6000 | 4950 | 1.0000 | 0.6246 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | without_amount | `719a9a9b98a4fad2bc7ac88232b65d60c28072a54bbbfb5cdcba556472deaa17` | 6000 | 4950 | 1.0000 | 0.4074 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | without_sequence | `719a9a9b98a4fad2bc7ac88232b65d60c28072a54bbbfb5cdcba556472deaa17` | 6000 | 4950 | 1.0000 | 0.7365 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | without_synchrony | `719a9a9b98a4fad2bc7ac88232b65d60c28072a54bbbfb5cdcba556472deaa17` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | without_funding | `719a9a9b98a4fad2bc7ac88232b65d60c28072a54bbbfb5cdcba556472deaa17` | 6000 | 4950 | 0.8074 | 0.1667 | 0.3600 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | none | `a29d06f84cc7fa08985371ecdcb95ce398c71555ef8129f816507fcef3475c81` | 6000 | 4950 | 1.0000 | 0.2032 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | without_timing | `a29d06f84cc7fa08985371ecdcb95ce398c71555ef8129f816507fcef3475c81` | 6000 | 4950 | 1.0000 | 0.6110 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | without_amount | `a29d06f84cc7fa08985371ecdcb95ce398c71555ef8129f816507fcef3475c81` | 6000 | 4950 | 1.0000 | 0.3846 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | without_sequence | `a29d06f84cc7fa08985371ecdcb95ce398c71555ef8129f816507fcef3475c81` | 6000 | 4950 | 1.0000 | 0.6772 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | without_synchrony | `a29d06f84cc7fa08985371ecdcb95ce398c71555ef8129f816507fcef3475c81` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | without_funding | `a29d06f84cc7fa08985371ecdcb95ce398c71555ef8129f816507fcef3475c81` | 6000 | 4950 | 0.7947 | 0.1667 | 0.3422 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 71 | none | `91175b5c1f3c0c832f68919f234e8a7d3abb05b0bb5971fe2d56ad3300ac85d1` | 6000 | 4950 | 1.0000 | 0.1947 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 71 | without_timing | `91175b5c1f3c0c832f68919f234e8a7d3abb05b0bb5971fe2d56ad3300ac85d1` | 6000 | 4950 | 1.0000 | 0.6456 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 71 | without_amount | `91175b5c1f3c0c832f68919f234e8a7d3abb05b0bb5971fe2d56ad3300ac85d1` | 6000 | 4950 | 1.0000 | 0.3825 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 71 | without_sequence | `91175b5c1f3c0c832f68919f234e8a7d3abb05b0bb5971fe2d56ad3300ac85d1` | 6000 | 4950 | 1.0000 | 0.6966 | 1.0000 | 0.0435 | 0.3757 |
| DedicatedPerOperator | IndependentWeighted | 71 | without_synchrony | `91175b5c1f3c0c832f68919f234e8a7d3abb05b0bb5971fe2d56ad3300ac85d1` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | IndependentWeighted | 71 | without_funding | `91175b5c1f3c0c832f68919f234e8a7d3abb05b0bb5971fe2d56ad3300ac85d1` | 6000 | 4950 | 0.8197 | 0.1667 | 0.3933 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 11 | none | `cb00645e3b0f09df296095ccf619398adf663348ea865a790ca0d0793506204a` | 6000 | 4950 | 0.9817 | 0.8139 | 0.7911 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 11 | without_timing | `cb00645e3b0f09df296095ccf619398adf663348ea865a790ca0d0793506204a` | 6000 | 4950 | 0.9988 | 0.9128 | 0.9578 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 11 | without_amount | `cb00645e3b0f09df296095ccf619398adf663348ea865a790ca0d0793506204a` | 6000 | 4950 | 0.9819 | 0.8303 | 0.7867 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 11 | without_sequence | `cb00645e3b0f09df296095ccf619398adf663348ea865a790ca0d0793506204a` | 6000 | 4950 | 0.9831 | 0.8520 | 0.8089 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 11 | without_synchrony | `cb00645e3b0f09df296095ccf619398adf663348ea865a790ca0d0793506204a` | 6000 | 4950 | 0.9971 | 0.6081 | 0.9000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 11 | without_funding | `cb00645e3b0f09df296095ccf619398adf663348ea865a790ca0d0793506204a` | 6000 | 4950 | 0.7568 | 0.2442 | 0.1400 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 23 | none | `928c2c8ce225d4f3ea4632368b3eb0c997355d083170a9f5f92f81cd8b381a3b` | 6000 | 4950 | 0.9803 | 0.7986 | 0.7600 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 23 | without_timing | `928c2c8ce225d4f3ea4632368b3eb0c997355d083170a9f5f92f81cd8b381a3b` | 6000 | 4950 | 0.9985 | 0.9156 | 0.9311 | 0.0994 | 0.5268 |
| DedicatedPerOperator | PersonaSession | 23 | without_amount | `928c2c8ce225d4f3ea4632368b3eb0c997355d083170a9f5f92f81cd8b381a3b` | 6000 | 4950 | 0.9803 | 0.8141 | 0.7622 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 23 | without_sequence | `928c2c8ce225d4f3ea4632368b3eb0c997355d083170a9f5f92f81cd8b381a3b` | 6000 | 4950 | 0.9824 | 0.8421 | 0.7933 | 0.0000 | 0.0654 |
| DedicatedPerOperator | PersonaSession | 23 | without_synchrony | `928c2c8ce225d4f3ea4632368b3eb0c997355d083170a9f5f92f81cd8b381a3b` | 6000 | 4950 | 0.9955 | 0.5964 | 0.8822 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 23 | without_funding | `928c2c8ce225d4f3ea4632368b3eb0c997355d083170a9f5f92f81cd8b381a3b` | 6000 | 4950 | 0.7456 | 0.2558 | 0.1444 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 37 | none | `711e68794110c3b9ea52c2044ebcbd0d16c5a93174d7bec2c417d0a8cba95864` | 6000 | 4950 | 0.9821 | 0.8018 | 0.7889 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 37 | without_timing | `711e68794110c3b9ea52c2044ebcbd0d16c5a93174d7bec2c417d0a8cba95864` | 6000 | 4950 | 0.9986 | 0.9395 | 0.9356 | 0.1720 | 0.6391 |
| DedicatedPerOperator | PersonaSession | 37 | without_amount | `711e68794110c3b9ea52c2044ebcbd0d16c5a93174d7bec2c417d0a8cba95864` | 6000 | 4950 | 0.9826 | 0.8297 | 0.7956 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 37 | without_sequence | `711e68794110c3b9ea52c2044ebcbd0d16c5a93174d7bec2c417d0a8cba95864` | 6000 | 4950 | 0.9839 | 0.8493 | 0.8200 | 0.0004 | 0.0946 |
| DedicatedPerOperator | PersonaSession | 37 | without_synchrony | `711e68794110c3b9ea52c2044ebcbd0d16c5a93174d7bec2c417d0a8cba95864` | 6000 | 4950 | 0.9956 | 0.6173 | 0.8933 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 37 | without_funding | `711e68794110c3b9ea52c2044ebcbd0d16c5a93174d7bec2c417d0a8cba95864` | 6000 | 4950 | 0.7531 | 0.2541 | 0.1600 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 51 | none | `67ea76982d09d814433e49aaf5f3f03d71f13b83bf75f27333bf8cd8e0b185ae` | 6000 | 4950 | 0.9812 | 0.7914 | 0.7778 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 51 | without_timing | `67ea76982d09d814433e49aaf5f3f03d71f13b83bf75f27333bf8cd8e0b185ae` | 6000 | 4950 | 0.9970 | 0.9100 | 0.9133 | 0.1720 | 0.6391 |
| DedicatedPerOperator | PersonaSession | 51 | without_amount | `67ea76982d09d814433e49aaf5f3f03d71f13b83bf75f27333bf8cd8e0b185ae` | 6000 | 4950 | 0.9817 | 0.8177 | 0.7756 | 0.0000 | 0.0654 |
| DedicatedPerOperator | PersonaSession | 51 | without_sequence | `67ea76982d09d814433e49aaf5f3f03d71f13b83bf75f27333bf8cd8e0b185ae` | 6000 | 4950 | 0.9835 | 0.8444 | 0.8222 | 0.0004 | 0.0946 |
| DedicatedPerOperator | PersonaSession | 51 | without_synchrony | `67ea76982d09d814433e49aaf5f3f03d71f13b83bf75f27333bf8cd8e0b185ae` | 6000 | 4950 | 0.9965 | 0.6233 | 0.9000 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 51 | without_funding | `67ea76982d09d814433e49aaf5f3f03d71f13b83bf75f27333bf8cd8e0b185ae` | 6000 | 4950 | 0.7632 | 0.2622 | 0.1333 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 71 | none | `ac95da4992f5cbd6580b753798e8f8ffac091fd07b4e4a79656c73239e52b7a6` | 6000 | 4950 | 0.9818 | 0.7922 | 0.7778 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 71 | without_timing | `ac95da4992f5cbd6580b753798e8f8ffac091fd07b4e4a79656c73239e52b7a6` | 6000 | 4950 | 0.9981 | 0.9146 | 0.9267 | 0.1720 | 0.6391 |
| DedicatedPerOperator | PersonaSession | 71 | without_amount | `ac95da4992f5cbd6580b753798e8f8ffac091fd07b4e4a79656c73239e52b7a6` | 6000 | 4950 | 0.9821 | 0.8224 | 0.7778 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 71 | without_sequence | `ac95da4992f5cbd6580b753798e8f8ffac091fd07b4e4a79656c73239e52b7a6` | 6000 | 4950 | 0.9828 | 0.8429 | 0.8022 | 0.0435 | 0.3757 |
| DedicatedPerOperator | PersonaSession | 71 | without_synchrony | `ac95da4992f5cbd6580b753798e8f8ffac091fd07b4e4a79656c73239e52b7a6` | 6000 | 4950 | 0.9969 | 0.6044 | 0.9022 | 0.0000 | 0.0000 |
| DedicatedPerOperator | PersonaSession | 71 | without_funding | `ac95da4992f5cbd6580b753798e8f8ffac091fd07b4e4a79656c73239e52b7a6` | 6000 | 4950 | 0.7465 | 0.2464 | 0.1289 | 0.0000 | 0.0000 |
| PooledMixedRounds | NaiveUniform | 11 | none | `3de7ba1aa41675808dbc0c2dad767d6b2db882918a96a1acb6d3bc682a7291e6` | 6000 | 4950 | 1.0000 | 0.6489 | 1.0000 | 0.2870 | 0.6687 |
| PooledMixedRounds | NaiveUniform | 11 | without_timing | `3de7ba1aa41675808dbc0c2dad767d6b2db882918a96a1acb6d3bc682a7291e6` | 6000 | 4950 | 1.0000 | 0.9657 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 11 | without_amount | `3de7ba1aa41675808dbc0c2dad767d6b2db882918a96a1acb6d3bc682a7291e6` | 6000 | 4950 | 1.0000 | 0.8094 | 1.0000 | 0.4568 | 0.7703 |
| PooledMixedRounds | NaiveUniform | 11 | without_sequence | `3de7ba1aa41675808dbc0c2dad767d6b2db882918a96a1acb6d3bc682a7291e6` | 6000 | 4950 | 1.0000 | 0.7705 | 1.0000 | 0.0435 | 0.3757 |
| PooledMixedRounds | NaiveUniform | 11 | without_synchrony | `3de7ba1aa41675808dbc0c2dad767d6b2db882918a96a1acb6d3bc682a7291e6` | 6000 | 4950 | 1.0000 | 0.5155 | 0.9933 | 0.0000 | 0.0000 |
| PooledMixedRounds | NaiveUniform | 11 | without_funding | `3de7ba1aa41675808dbc0c2dad767d6b2db882918a96a1acb6d3bc682a7291e6` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 23 | none | `1176deacb5de3b342dd06ea340b36f1bc00241457c3ba9146e9fa895f8872827` | 6000 | 4950 | 1.0000 | 0.6623 | 1.0000 | 0.1852 | 0.5487 |
| PooledMixedRounds | NaiveUniform | 23 | without_timing | `1176deacb5de3b342dd06ea340b36f1bc00241457c3ba9146e9fa895f8872827` | 6000 | 4950 | 1.0000 | 0.9626 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 23 | without_amount | `1176deacb5de3b342dd06ea340b36f1bc00241457c3ba9146e9fa895f8872827` | 6000 | 4950 | 1.0000 | 0.8050 | 1.0000 | 0.4568 | 0.7703 |
| PooledMixedRounds | NaiveUniform | 23 | without_sequence | `1176deacb5de3b342dd06ea340b36f1bc00241457c3ba9146e9fa895f8872827` | 6000 | 4950 | 1.0000 | 0.7792 | 1.0000 | 0.0435 | 0.3757 |
| PooledMixedRounds | NaiveUniform | 23 | without_synchrony | `1176deacb5de3b342dd06ea340b36f1bc00241457c3ba9146e9fa895f8872827` | 6000 | 4950 | 1.0000 | 0.5263 | 0.9978 | 0.0000 | 0.0000 |
| PooledMixedRounds | NaiveUniform | 23 | without_funding | `1176deacb5de3b342dd06ea340b36f1bc00241457c3ba9146e9fa895f8872827` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 37 | none | `b5f0bb5c69ab995ab00c65569def104eb97539dcc4cf5564d67fbac0cb9f35d4` | 6000 | 4950 | 1.0000 | 0.6494 | 1.0000 | 0.3333 | 0.7281 |
| PooledMixedRounds | NaiveUniform | 37 | without_timing | `b5f0bb5c69ab995ab00c65569def104eb97539dcc4cf5564d67fbac0cb9f35d4` | 6000 | 4950 | 1.0000 | 0.9484 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 37 | without_amount | `b5f0bb5c69ab995ab00c65569def104eb97539dcc4cf5564d67fbac0cb9f35d4` | 6000 | 4950 | 1.0000 | 0.7993 | 1.0000 | 0.4568 | 0.7703 |
| PooledMixedRounds | NaiveUniform | 37 | without_sequence | `b5f0bb5c69ab995ab00c65569def104eb97539dcc4cf5564d67fbac0cb9f35d4` | 6000 | 4950 | 1.0000 | 0.7909 | 1.0000 | 0.0435 | 0.3757 |
| PooledMixedRounds | NaiveUniform | 37 | without_synchrony | `b5f0bb5c69ab995ab00c65569def104eb97539dcc4cf5564d67fbac0cb9f35d4` | 6000 | 4950 | 1.0000 | 0.5282 | 0.9911 | 0.0000 | 0.0000 |
| PooledMixedRounds | NaiveUniform | 37 | without_funding | `b5f0bb5c69ab995ab00c65569def104eb97539dcc4cf5564d67fbac0cb9f35d4` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 51 | none | `55feb29f9f5f044364acf0fadea9bb2471ae8768aafe5b3cef213b42ef7b4753` | 6000 | 4950 | 1.0000 | 0.6508 | 1.0000 | 0.1852 | 0.5487 |
| PooledMixedRounds | NaiveUniform | 51 | without_timing | `55feb29f9f5f044364acf0fadea9bb2471ae8768aafe5b3cef213b42ef7b4753` | 6000 | 4950 | 1.0000 | 0.9424 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 51 | without_amount | `55feb29f9f5f044364acf0fadea9bb2471ae8768aafe5b3cef213b42ef7b4753` | 6000 | 4950 | 1.0000 | 0.7937 | 1.0000 | 0.4568 | 0.7703 |
| PooledMixedRounds | NaiveUniform | 51 | without_sequence | `55feb29f9f5f044364acf0fadea9bb2471ae8768aafe5b3cef213b42ef7b4753` | 6000 | 4950 | 1.0000 | 0.7686 | 1.0000 | 0.0435 | 0.3757 |
| PooledMixedRounds | NaiveUniform | 51 | without_synchrony | `55feb29f9f5f044364acf0fadea9bb2471ae8768aafe5b3cef213b42ef7b4753` | 6000 | 4950 | 1.0000 | 0.5220 | 0.9956 | 0.0000 | 0.0000 |
| PooledMixedRounds | NaiveUniform | 51 | without_funding | `55feb29f9f5f044364acf0fadea9bb2471ae8768aafe5b3cef213b42ef7b4753` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 71 | none | `82fedda7248aefd58770a4692fbaf53c26cfa7ca23b24d69ea37d672aaabf35c` | 6000 | 4950 | 1.0000 | 0.6579 | 1.0000 | 0.3333 | 0.7281 |
| PooledMixedRounds | NaiveUniform | 71 | without_timing | `82fedda7248aefd58770a4692fbaf53c26cfa7ca23b24d69ea37d672aaabf35c` | 6000 | 4950 | 1.0000 | 0.9564 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | NaiveUniform | 71 | without_amount | `82fedda7248aefd58770a4692fbaf53c26cfa7ca23b24d69ea37d672aaabf35c` | 6000 | 4950 | 1.0000 | 0.8094 | 1.0000 | 0.4568 | 0.7703 |
| PooledMixedRounds | NaiveUniform | 71 | without_sequence | `82fedda7248aefd58770a4692fbaf53c26cfa7ca23b24d69ea37d672aaabf35c` | 6000 | 4950 | 1.0000 | 0.7712 | 1.0000 | 0.0435 | 0.3757 |
| PooledMixedRounds | NaiveUniform | 71 | without_synchrony | `82fedda7248aefd58770a4692fbaf53c26cfa7ca23b24d69ea37d672aaabf35c` | 6000 | 4950 | 1.0000 | 0.5178 | 0.9956 | 0.0000 | 0.0000 |
| PooledMixedRounds | NaiveUniform | 71 | without_funding | `82fedda7248aefd58770a4692fbaf53c26cfa7ca23b24d69ea37d672aaabf35c` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledMixedRounds | IndependentWeighted | 11 | none | `2cb224d6b0ed5ab7278999514b5209986de72e6d077c982ba5d65741c8f1f89e` | 6000 | 4950 | 0.7075 | 0.1697 | 0.2133 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 11 | without_timing | `2cb224d6b0ed5ab7278999514b5209986de72e6d077c982ba5d65741c8f1f89e` | 6000 | 4950 | 0.7154 | 0.2084 | 0.1933 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 11 | without_amount | `2cb224d6b0ed5ab7278999514b5209986de72e6d077c982ba5d65741c8f1f89e` | 6000 | 4950 | 0.7093 | 0.1881 | 0.2111 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 11 | without_sequence | `2cb224d6b0ed5ab7278999514b5209986de72e6d077c982ba5d65741c8f1f89e` | 6000 | 4950 | 0.7129 | 0.2147 | 0.2022 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 11 | without_synchrony | `2cb224d6b0ed5ab7278999514b5209986de72e6d077c982ba5d65741c8f1f89e` | 6000 | 4950 | 0.7076 | 0.1667 | 0.2089 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 11 | without_funding | `2cb224d6b0ed5ab7278999514b5209986de72e6d077c982ba5d65741c8f1f89e` | 6000 | 4950 | 0.7998 | 0.1667 | 0.3622 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 23 | none | `d120c3c0ba30b8b32f9aaffb092c9cfd184426b5c8222196be6eebf3dbb25ea2` | 6000 | 4950 | 0.7127 | 0.1709 | 0.2000 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 23 | without_timing | `d120c3c0ba30b8b32f9aaffb092c9cfd184426b5c8222196be6eebf3dbb25ea2` | 6000 | 4950 | 0.7195 | 0.2184 | 0.2111 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 23 | without_amount | `d120c3c0ba30b8b32f9aaffb092c9cfd184426b5c8222196be6eebf3dbb25ea2` | 6000 | 4950 | 0.7138 | 0.1917 | 0.1911 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 23 | without_sequence | `d120c3c0ba30b8b32f9aaffb092c9cfd184426b5c8222196be6eebf3dbb25ea2` | 6000 | 4950 | 0.7248 | 0.2182 | 0.2111 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 23 | without_synchrony | `d120c3c0ba30b8b32f9aaffb092c9cfd184426b5c8222196be6eebf3dbb25ea2` | 6000 | 4950 | 0.7130 | 0.1667 | 0.2022 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 23 | without_funding | `d120c3c0ba30b8b32f9aaffb092c9cfd184426b5c8222196be6eebf3dbb25ea2` | 6000 | 4950 | 0.7957 | 0.1667 | 0.3511 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 37 | none | `d41a4d4be3cd539dd87592ef45eb8fc7ae7041a01b763e523766ed388cf6025a` | 6000 | 4950 | 0.7193 | 0.1726 | 0.2089 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 37 | without_timing | `d41a4d4be3cd539dd87592ef45eb8fc7ae7041a01b763e523766ed388cf6025a` | 6000 | 4950 | 0.7267 | 0.2193 | 0.2200 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 37 | without_amount | `d41a4d4be3cd539dd87592ef45eb8fc7ae7041a01b763e523766ed388cf6025a` | 6000 | 4950 | 0.7187 | 0.1976 | 0.2156 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 37 | without_sequence | `d41a4d4be3cd539dd87592ef45eb8fc7ae7041a01b763e523766ed388cf6025a` | 6000 | 4950 | 0.7247 | 0.2260 | 0.2133 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 37 | without_synchrony | `d41a4d4be3cd539dd87592ef45eb8fc7ae7041a01b763e523766ed388cf6025a` | 6000 | 4950 | 0.7194 | 0.1667 | 0.2089 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 37 | without_funding | `d41a4d4be3cd539dd87592ef45eb8fc7ae7041a01b763e523766ed388cf6025a` | 6000 | 4950 | 0.8074 | 0.1667 | 0.3600 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 51 | none | `c348fae0347b154f0d7f534ddf3bfd81e6237b68e54d41bf76e5a4573e278f90` | 6000 | 4950 | 0.7122 | 0.1708 | 0.2089 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 51 | without_timing | `c348fae0347b154f0d7f534ddf3bfd81e6237b68e54d41bf76e5a4573e278f90` | 6000 | 4950 | 0.7225 | 0.2168 | 0.2289 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 51 | without_amount | `c348fae0347b154f0d7f534ddf3bfd81e6237b68e54d41bf76e5a4573e278f90` | 6000 | 4950 | 0.7128 | 0.1971 | 0.2133 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 51 | without_sequence | `c348fae0347b154f0d7f534ddf3bfd81e6237b68e54d41bf76e5a4573e278f90` | 6000 | 4950 | 0.7189 | 0.2212 | 0.2178 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 51 | without_synchrony | `c348fae0347b154f0d7f534ddf3bfd81e6237b68e54d41bf76e5a4573e278f90` | 6000 | 4950 | 0.7118 | 0.1667 | 0.2111 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 51 | without_funding | `c348fae0347b154f0d7f534ddf3bfd81e6237b68e54d41bf76e5a4573e278f90` | 6000 | 4950 | 0.7947 | 0.1667 | 0.3422 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 71 | none | `78ee28027220e813a2316a20232a1529c1701c5e4c2043803ec0bc0668bfaccc` | 6000 | 4950 | 0.7036 | 0.1702 | 0.1933 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 71 | without_timing | `78ee28027220e813a2316a20232a1529c1701c5e4c2043803ec0bc0668bfaccc` | 6000 | 4950 | 0.7033 | 0.2113 | 0.2022 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 71 | without_amount | `78ee28027220e813a2316a20232a1529c1701c5e4c2043803ec0bc0668bfaccc` | 6000 | 4950 | 0.7018 | 0.1912 | 0.2044 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 71 | without_sequence | `78ee28027220e813a2316a20232a1529c1701c5e4c2043803ec0bc0668bfaccc` | 6000 | 4950 | 0.7121 | 0.2196 | 0.2089 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 71 | without_synchrony | `78ee28027220e813a2316a20232a1529c1701c5e4c2043803ec0bc0668bfaccc` | 6000 | 4950 | 0.7039 | 0.1667 | 0.1933 | 0.0000 | 0.0000 |
| PooledMixedRounds | IndependentWeighted | 71 | without_funding | `78ee28027220e813a2316a20232a1529c1701c5e4c2043803ec0bc0668bfaccc` | 6000 | 4950 | 0.8197 | 0.1667 | 0.3933 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 11 | none | `7f6429699673acfe1ae45141f088b72702b671726f8c48468447fde7323e38f5` | 6000 | 4950 | 0.6735 | 0.1188 | 0.1178 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 11 | without_timing | `7f6429699673acfe1ae45141f088b72702b671726f8c48468447fde7323e38f5` | 6000 | 4950 | 0.6959 | 0.2569 | 0.1600 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 11 | without_amount | `7f6429699673acfe1ae45141f088b72702b671726f8c48468447fde7323e38f5` | 6000 | 4950 | 0.6763 | 0.0849 | 0.1178 | -0.0003 | 0.0668 |
| PooledMixedRounds | PersonaSession | 11 | without_sequence | `7f6429699673acfe1ae45141f088b72702b671726f8c48468447fde7323e38f5` | 6000 | 4950 | 0.6861 | 0.0747 | 0.1244 | 0.0005 | 0.1885 |
| PooledMixedRounds | PersonaSession | 11 | without_synchrony | `7f6429699673acfe1ae45141f088b72702b671726f8c48468447fde7323e38f5` | 6000 | 4950 | 0.6770 | 0.2176 | 0.1333 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 11 | without_funding | `7f6429699673acfe1ae45141f088b72702b671726f8c48468447fde7323e38f5` | 6000 | 4950 | 0.7568 | 0.2442 | 0.1400 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 23 | none | `d2599158a30ea0187e02f61a221b9b6aeac232fd017400334ec2ccaf51db3892` | 6000 | 4950 | 0.6824 | 0.1515 | 0.1489 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 23 | without_timing | `d2599158a30ea0187e02f61a221b9b6aeac232fd017400334ec2ccaf51db3892` | 6000 | 4950 | 0.7065 | 0.2441 | 0.1711 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 23 | without_amount | `d2599158a30ea0187e02f61a221b9b6aeac232fd017400334ec2ccaf51db3892` | 6000 | 4950 | 0.6838 | 0.0997 | 0.1444 | 0.0029 | 0.1877 |
| PooledMixedRounds | PersonaSession | 23 | without_sequence | `d2599158a30ea0187e02f61a221b9b6aeac232fd017400334ec2ccaf51db3892` | 6000 | 4950 | 0.6954 | 0.0871 | 0.1422 | 0.0038 | 0.2158 |
| PooledMixedRounds | PersonaSession | 23 | without_synchrony | `d2599158a30ea0187e02f61a221b9b6aeac232fd017400334ec2ccaf51db3892` | 6000 | 4950 | 0.6845 | 0.2352 | 0.1511 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 23 | without_funding | `d2599158a30ea0187e02f61a221b9b6aeac232fd017400334ec2ccaf51db3892` | 6000 | 4950 | 0.7456 | 0.2558 | 0.1444 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 37 | none | `6c7ede515f7bd818e7abf71072b377b4d7aba60e52fe673d43d1bf874dc85799` | 6000 | 4950 | 0.6815 | 0.1330 | 0.1356 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 37 | without_timing | `6c7ede515f7bd818e7abf71072b377b4d7aba60e52fe673d43d1bf874dc85799` | 6000 | 4950 | 0.7026 | 0.2530 | 0.2067 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 37 | without_amount | `6c7ede515f7bd818e7abf71072b377b4d7aba60e52fe673d43d1bf874dc85799` | 6000 | 4950 | 0.6839 | 0.0859 | 0.1400 | -0.0004 | 0.0912 |
| PooledMixedRounds | PersonaSession | 37 | without_sequence | `6c7ede515f7bd818e7abf71072b377b4d7aba60e52fe673d43d1bf874dc85799` | 6000 | 4950 | 0.6964 | 0.0783 | 0.1378 | 0.0001 | 0.1821 |
| PooledMixedRounds | PersonaSession | 37 | without_synchrony | `6c7ede515f7bd818e7abf71072b377b4d7aba60e52fe673d43d1bf874dc85799` | 6000 | 4950 | 0.6823 | 0.2277 | 0.1378 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 37 | without_funding | `6c7ede515f7bd818e7abf71072b377b4d7aba60e52fe673d43d1bf874dc85799` | 6000 | 4950 | 0.7531 | 0.2541 | 0.1600 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 51 | none | `a83838dd3e2ee9f004a2f6ede82b995b0b079ae246c0c59d9b38037c40799418` | 6000 | 4950 | 0.6849 | 0.1566 | 0.1489 | 0.0000 | 0.0654 |
| PooledMixedRounds | PersonaSession | 51 | without_timing | `a83838dd3e2ee9f004a2f6ede82b995b0b079ae246c0c59d9b38037c40799418` | 6000 | 4950 | 0.7073 | 0.2483 | 0.2156 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 51 | without_amount | `a83838dd3e2ee9f004a2f6ede82b995b0b079ae246c0c59d9b38037c40799418` | 6000 | 4950 | 0.6888 | 0.1071 | 0.1511 | 0.0010 | 0.1357 |
| PooledMixedRounds | PersonaSession | 51 | without_sequence | `a83838dd3e2ee9f004a2f6ede82b995b0b079ae246c0c59d9b38037c40799418` | 6000 | 4950 | 0.6952 | 0.0943 | 0.1444 | 0.0046 | 0.2155 |
| PooledMixedRounds | PersonaSession | 51 | without_synchrony | `a83838dd3e2ee9f004a2f6ede82b995b0b079ae246c0c59d9b38037c40799418` | 6000 | 4950 | 0.6873 | 0.2224 | 0.1467 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 51 | without_funding | `a83838dd3e2ee9f004a2f6ede82b995b0b079ae246c0c59d9b38037c40799418` | 6000 | 4950 | 0.7632 | 0.2622 | 0.1333 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 71 | none | `edfbcbf02a123604389ce5b4abb651fd9d00b237adb714cab978ed901a41929a` | 6000 | 4950 | 0.6531 | 0.1457 | 0.1333 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 71 | without_timing | `edfbcbf02a123604389ce5b4abb651fd9d00b237adb714cab978ed901a41929a` | 6000 | 4950 | 0.6759 | 0.2502 | 0.1867 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 71 | without_amount | `edfbcbf02a123604389ce5b4abb651fd9d00b237adb714cab978ed901a41929a` | 6000 | 4950 | 0.6547 | 0.0949 | 0.1333 | 0.0018 | 0.2064 |
| PooledMixedRounds | PersonaSession | 71 | without_sequence | `edfbcbf02a123604389ce5b4abb651fd9d00b237adb714cab978ed901a41929a` | 6000 | 4950 | 0.6636 | 0.0918 | 0.1400 | 0.0063 | 0.2244 |
| PooledMixedRounds | PersonaSession | 71 | without_synchrony | `edfbcbf02a123604389ce5b4abb651fd9d00b237adb714cab978ed901a41929a` | 6000 | 4950 | 0.6547 | 0.2119 | 0.1422 | 0.0000 | 0.0000 |
| PooledMixedRounds | PersonaSession | 71 | without_funding | `edfbcbf02a123604389ce5b4abb651fd9d00b237adb714cab978ed901a41929a` | 6000 | 4950 | 0.7465 | 0.2464 | 0.1289 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | NaiveUniform | 11 | none | `a41e62eca97b924ae981616ab93241952ff5cf90aa271c80beeff0bef36f0d3f` | 6000 | 4950 | 1.0000 | 0.6667 | 1.0000 | 0.2870 | 0.6687 |
| PooledPerOperatorRounds | NaiveUniform | 11 | without_timing | `a41e62eca97b924ae981616ab93241952ff5cf90aa271c80beeff0bef36f0d3f` | 6000 | 4950 | 1.0000 | 0.9804 | 1.0000 | 0.5926 | 0.8582 |
| PooledPerOperatorRounds | NaiveUniform | 11 | without_amount | `a41e62eca97b924ae981616ab93241952ff5cf90aa271c80beeff0bef36f0d3f` | 6000 | 4950 | 1.0000 | 0.8349 | 1.0000 | 0.4568 | 0.7703 |
| PooledPerOperatorRounds | NaiveUniform | 11 | without_sequence | `a41e62eca97b924ae981616ab93241952ff5cf90aa271c80beeff0bef36f0d3f` | 6000 | 4950 | 1.0000 | 0.7937 | 1.0000 | 0.0435 | 0.3757 |
| PooledPerOperatorRounds | NaiveUniform | 11 | without_synchrony | `a41e62eca97b924ae981616ab93241952ff5cf90aa271c80beeff0bef36f0d3f` | 6000 | 4950 | 1.0000 | 0.5164 | 1.0000 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | NaiveUniform | 11 | without_funding | `a41e62eca97b924ae981616ab93241952ff5cf90aa271c80beeff0bef36f0d3f` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 23 | none | `a4cfc1119d0bb91da34f2ba0372e8c1a38b559d980011c314d569fd037d9e463` | 6000 | 4950 | 1.0000 | 0.6429 | 1.0000 | 0.1852 | 0.5487 |
| PooledPerOperatorRounds | NaiveUniform | 23 | without_timing | `a4cfc1119d0bb91da34f2ba0372e8c1a38b559d980011c314d569fd037d9e463` | 6000 | 4950 | 1.0000 | 0.9544 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 23 | without_amount | `a4cfc1119d0bb91da34f2ba0372e8c1a38b559d980011c314d569fd037d9e463` | 6000 | 4950 | 1.0000 | 0.7937 | 1.0000 | 0.4568 | 0.7703 |
| PooledPerOperatorRounds | NaiveUniform | 23 | without_sequence | `a4cfc1119d0bb91da34f2ba0372e8c1a38b559d980011c314d569fd037d9e463` | 6000 | 4950 | 1.0000 | 0.7813 | 1.0000 | 0.0435 | 0.3757 |
| PooledPerOperatorRounds | NaiveUniform | 23 | without_synchrony | `a4cfc1119d0bb91da34f2ba0372e8c1a38b559d980011c314d569fd037d9e463` | 6000 | 4950 | 1.0000 | 0.5205 | 0.9978 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | NaiveUniform | 23 | without_funding | `a4cfc1119d0bb91da34f2ba0372e8c1a38b559d980011c314d569fd037d9e463` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 37 | none | `93ff0a2e68e9e8525e7233d61aee9fe3ecb5469582af9ebfafbfb9adfb1cca61` | 6000 | 4950 | 1.0000 | 0.6272 | 1.0000 | 0.2870 | 0.6687 |
| PooledPerOperatorRounds | NaiveUniform | 37 | without_timing | `93ff0a2e68e9e8525e7233d61aee9fe3ecb5469582af9ebfafbfb9adfb1cca61` | 6000 | 4950 | 1.0000 | 0.9231 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 37 | without_amount | `93ff0a2e68e9e8525e7233d61aee9fe3ecb5469582af9ebfafbfb9adfb1cca61` | 6000 | 4950 | 1.0000 | 0.7765 | 1.0000 | 0.4568 | 0.7703 |
| PooledPerOperatorRounds | NaiveUniform | 37 | without_sequence | `93ff0a2e68e9e8525e7233d61aee9fe3ecb5469582af9ebfafbfb9adfb1cca61` | 6000 | 4950 | 1.0000 | 0.7826 | 1.0000 | 0.0435 | 0.3757 |
| PooledPerOperatorRounds | NaiveUniform | 37 | without_synchrony | `93ff0a2e68e9e8525e7233d61aee9fe3ecb5469582af9ebfafbfb9adfb1cca61` | 6000 | 4950 | 1.0000 | 0.5269 | 1.0000 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | NaiveUniform | 37 | without_funding | `93ff0a2e68e9e8525e7233d61aee9fe3ecb5469582af9ebfafbfb9adfb1cca61` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 51 | none | `c500a6d3d88125d2f7e21c748acf0ce0236899d991bf2d5a46fc2d4e455e2db2` | 6000 | 4950 | 1.0000 | 0.6584 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 51 | without_timing | `c500a6d3d88125d2f7e21c748acf0ce0236899d991bf2d5a46fc2d4e455e2db2` | 6000 | 4950 | 1.0000 | 0.9474 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 51 | without_amount | `c500a6d3d88125d2f7e21c748acf0ce0236899d991bf2d5a46fc2d4e455e2db2` | 6000 | 4950 | 1.0000 | 0.8057 | 1.0000 | 0.4568 | 0.7703 |
| PooledPerOperatorRounds | NaiveUniform | 51 | without_sequence | `c500a6d3d88125d2f7e21c748acf0ce0236899d991bf2d5a46fc2d4e455e2db2` | 6000 | 4950 | 1.0000 | 0.7660 | 1.0000 | 0.0435 | 0.3757 |
| PooledPerOperatorRounds | NaiveUniform | 51 | without_synchrony | `c500a6d3d88125d2f7e21c748acf0ce0236899d991bf2d5a46fc2d4e455e2db2` | 6000 | 4950 | 1.0000 | 0.5248 | 1.0000 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | NaiveUniform | 51 | without_funding | `c500a6d3d88125d2f7e21c748acf0ce0236899d991bf2d5a46fc2d4e455e2db2` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 71 | none | `54d47829c448cb63c7df5038faa5ffcce2c25e0082f8bfe18397a16a15c62f41` | 6000 | 4950 | 1.0000 | 0.6410 | 1.0000 | 0.2870 | 0.6687 |
| PooledPerOperatorRounds | NaiveUniform | 71 | without_timing | `54d47829c448cb63c7df5038faa5ffcce2c25e0082f8bfe18397a16a15c62f41` | 6000 | 4950 | 1.0000 | 0.9585 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | NaiveUniform | 71 | without_amount | `54d47829c448cb63c7df5038faa5ffcce2c25e0082f8bfe18397a16a15c62f41` | 6000 | 4950 | 1.0000 | 0.7874 | 1.0000 | 0.4568 | 0.7703 |
| PooledPerOperatorRounds | NaiveUniform | 71 | without_sequence | `54d47829c448cb63c7df5038faa5ffcce2c25e0082f8bfe18397a16a15c62f41` | 6000 | 4950 | 1.0000 | 0.7792 | 1.0000 | 0.0435 | 0.3757 |
| PooledPerOperatorRounds | NaiveUniform | 71 | without_synchrony | `54d47829c448cb63c7df5038faa5ffcce2c25e0082f8bfe18397a16a15c62f41` | 6000 | 4950 | 1.0000 | 0.5373 | 1.0000 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | NaiveUniform | 71 | without_funding | `54d47829c448cb63c7df5038faa5ffcce2c25e0082f8bfe18397a16a15c62f41` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | none | `bc3e1ec79d467b3e2222beae2391e6fdea21dbc2d9f5a7d28695a267da6b7dda` | 6000 | 4950 | 0.8269 | 0.1699 | 0.3956 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | without_timing | `bc3e1ec79d467b3e2222beae2391e6fdea21dbc2d9f5a7d28695a267da6b7dda` | 6000 | 4950 | 0.8379 | 0.2227 | 0.3911 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | without_amount | `bc3e1ec79d467b3e2222beae2391e6fdea21dbc2d9f5a7d28695a267da6b7dda` | 6000 | 4950 | 0.8322 | 0.1941 | 0.3933 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | without_sequence | `bc3e1ec79d467b3e2222beae2391e6fdea21dbc2d9f5a7d28695a267da6b7dda` | 6000 | 4950 | 0.8357 | 0.2371 | 0.3956 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | without_synchrony | `bc3e1ec79d467b3e2222beae2391e6fdea21dbc2d9f5a7d28695a267da6b7dda` | 6000 | 4950 | 0.8271 | 0.1667 | 0.3956 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | without_funding | `bc3e1ec79d467b3e2222beae2391e6fdea21dbc2d9f5a7d28695a267da6b7dda` | 6000 | 4950 | 0.7998 | 0.1667 | 0.3622 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | none | `07f415f49d49bdbae7730b7006f018d5ae5ad6d2173d59af6f4c2200867b53de` | 6000 | 4950 | 0.8269 | 0.1716 | 0.3822 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | without_timing | `07f415f49d49bdbae7730b7006f018d5ae5ad6d2173d59af6f4c2200867b53de` | 6000 | 4950 | 0.8365 | 0.2316 | 0.3800 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | without_amount | `07f415f49d49bdbae7730b7006f018d5ae5ad6d2173d59af6f4c2200867b53de` | 6000 | 4950 | 0.8284 | 0.2021 | 0.3867 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | without_sequence | `07f415f49d49bdbae7730b7006f018d5ae5ad6d2173d59af6f4c2200867b53de` | 6000 | 4950 | 0.8399 | 0.2385 | 0.3778 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | without_synchrony | `07f415f49d49bdbae7730b7006f018d5ae5ad6d2173d59af6f4c2200867b53de` | 6000 | 4950 | 0.8276 | 0.1667 | 0.3889 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | without_funding | `07f415f49d49bdbae7730b7006f018d5ae5ad6d2173d59af6f4c2200867b53de` | 6000 | 4950 | 0.7957 | 0.1667 | 0.3511 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | none | `38b37e6fe664697c21037bf74ab79e98773f4ab35de2c16ec204b7bf11c56c99` | 6000 | 4950 | 0.8519 | 0.1715 | 0.4222 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | without_timing | `38b37e6fe664697c21037bf74ab79e98773f4ab35de2c16ec204b7bf11c56c99` | 6000 | 4950 | 0.8583 | 0.2308 | 0.4444 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | without_amount | `38b37e6fe664697c21037bf74ab79e98773f4ab35de2c16ec204b7bf11c56c99` | 6000 | 4950 | 0.8548 | 0.2001 | 0.4289 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | without_sequence | `38b37e6fe664697c21037bf74ab79e98773f4ab35de2c16ec204b7bf11c56c99` | 6000 | 4950 | 0.8623 | 0.2437 | 0.4222 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | without_synchrony | `38b37e6fe664697c21037bf74ab79e98773f4ab35de2c16ec204b7bf11c56c99` | 6000 | 4950 | 0.8519 | 0.1667 | 0.4200 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | without_funding | `38b37e6fe664697c21037bf74ab79e98773f4ab35de2c16ec204b7bf11c56c99` | 6000 | 4950 | 0.8074 | 0.1667 | 0.3600 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | none | `c58e4492bdbf494e67277e38a0e9c47fe1dd144d1c28c45fc0aef4c51bb7944e` | 6000 | 4950 | 0.8265 | 0.1728 | 0.3400 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | without_timing | `c58e4492bdbf494e67277e38a0e9c47fe1dd144d1c28c45fc0aef4c51bb7944e` | 6000 | 4950 | 0.8348 | 0.2292 | 0.3622 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | without_amount | `c58e4492bdbf494e67277e38a0e9c47fe1dd144d1c28c45fc0aef4c51bb7944e` | 6000 | 4950 | 0.8270 | 0.2025 | 0.3378 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | without_sequence | `c58e4492bdbf494e67277e38a0e9c47fe1dd144d1c28c45fc0aef4c51bb7944e` | 6000 | 4950 | 0.8408 | 0.2398 | 0.3667 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | without_synchrony | `c58e4492bdbf494e67277e38a0e9c47fe1dd144d1c28c45fc0aef4c51bb7944e` | 6000 | 4950 | 0.8266 | 0.1667 | 0.3422 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | without_funding | `c58e4492bdbf494e67277e38a0e9c47fe1dd144d1c28c45fc0aef4c51bb7944e` | 6000 | 4950 | 0.7947 | 0.1667 | 0.3422 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | none | `004b8a11c9f4124052f11c9df64d2e01e04436281493a3e5dbcb49002aef935a` | 6000 | 4950 | 0.8570 | 0.1703 | 0.4067 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | without_timing | `004b8a11c9f4124052f11c9df64d2e01e04436281493a3e5dbcb49002aef935a` | 6000 | 4950 | 0.8662 | 0.2289 | 0.4022 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | without_amount | `004b8a11c9f4124052f11c9df64d2e01e04436281493a3e5dbcb49002aef935a` | 6000 | 4950 | 0.8572 | 0.1969 | 0.4044 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | without_sequence | `004b8a11c9f4124052f11c9df64d2e01e04436281493a3e5dbcb49002aef935a` | 6000 | 4950 | 0.8689 | 0.2363 | 0.4000 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | without_synchrony | `004b8a11c9f4124052f11c9df64d2e01e04436281493a3e5dbcb49002aef935a` | 6000 | 4950 | 0.8572 | 0.1667 | 0.4156 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | without_funding | `004b8a11c9f4124052f11c9df64d2e01e04436281493a3e5dbcb49002aef935a` | 6000 | 4950 | 0.8197 | 0.1667 | 0.3933 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 11 | none | `c2325ccacc1e2022b78dad3df9a5759b0f6c5d1920ffb7a84b9031894a6b00c5` | 6000 | 4950 | 0.7952 | 0.2490 | 0.2311 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 11 | without_timing | `c2325ccacc1e2022b78dad3df9a5759b0f6c5d1920ffb7a84b9031894a6b00c5` | 6000 | 4950 | 0.8264 | 0.3649 | 0.3222 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 11 | without_amount | `c2325ccacc1e2022b78dad3df9a5759b0f6c5d1920ffb7a84b9031894a6b00c5` | 6000 | 4950 | 0.7970 | 0.1475 | 0.2311 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 11 | without_sequence | `c2325ccacc1e2022b78dad3df9a5759b0f6c5d1920ffb7a84b9031894a6b00c5` | 6000 | 4950 | 0.8062 | 0.1143 | 0.2356 | 0.0020 | 0.1502 |
| PooledPerOperatorRounds | PersonaSession | 11 | without_synchrony | `c2325ccacc1e2022b78dad3df9a5759b0f6c5d1920ffb7a84b9031894a6b00c5` | 6000 | 4950 | 0.8010 | 0.2500 | 0.2489 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 11 | without_funding | `c2325ccacc1e2022b78dad3df9a5759b0f6c5d1920ffb7a84b9031894a6b00c5` | 6000 | 4950 | 0.7568 | 0.2442 | 0.1400 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 23 | none | `ace4966b90845dd327cbe9958a4084d9476d837323788ad5740801494bf3604b` | 6000 | 4950 | 0.7770 | 0.2563 | 0.2244 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 23 | without_timing | `ace4966b90845dd327cbe9958a4084d9476d837323788ad5740801494bf3604b` | 6000 | 4950 | 0.8106 | 0.3414 | 0.3267 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 23 | without_amount | `ace4966b90845dd327cbe9958a4084d9476d837323788ad5740801494bf3604b` | 6000 | 4950 | 0.7766 | 0.1877 | 0.2244 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 23 | without_sequence | `ace4966b90845dd327cbe9958a4084d9476d837323788ad5740801494bf3604b` | 6000 | 4950 | 0.7976 | 0.1482 | 0.2467 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 23 | without_synchrony | `ace4966b90845dd327cbe9958a4084d9476d837323788ad5740801494bf3604b` | 6000 | 4950 | 0.7814 | 0.2540 | 0.2311 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 23 | without_funding | `ace4966b90845dd327cbe9958a4084d9476d837323788ad5740801494bf3604b` | 6000 | 4950 | 0.7456 | 0.2558 | 0.1444 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 37 | none | `509a33763904235eef0f46f7f99276ed71787aaa1f0e64d7f9902b0cd2a8e17a` | 6000 | 4950 | 0.8056 | 0.2657 | 0.2511 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 37 | without_timing | `509a33763904235eef0f46f7f99276ed71787aaa1f0e64d7f9902b0cd2a8e17a` | 6000 | 4950 | 0.8423 | 0.3887 | 0.3800 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 37 | without_amount | `509a33763904235eef0f46f7f99276ed71787aaa1f0e64d7f9902b0cd2a8e17a` | 6000 | 4950 | 0.8092 | 0.1583 | 0.2489 | -0.0003 | 0.0668 |
| PooledPerOperatorRounds | PersonaSession | 37 | without_sequence | `509a33763904235eef0f46f7f99276ed71787aaa1f0e64d7f9902b0cd2a8e17a` | 6000 | 4950 | 0.8205 | 0.1186 | 0.2644 | -0.0009 | 0.1138 |
| PooledPerOperatorRounds | PersonaSession | 37 | without_synchrony | `509a33763904235eef0f46f7f99276ed71787aaa1f0e64d7f9902b0cd2a8e17a` | 6000 | 4950 | 0.8075 | 0.2518 | 0.2533 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 37 | without_funding | `509a33763904235eef0f46f7f99276ed71787aaa1f0e64d7f9902b0cd2a8e17a` | 6000 | 4950 | 0.7531 | 0.2541 | 0.1600 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 51 | none | `0f215fe8a9b82707703b29296508ed6402057a2376dce85c316f9c61016f8275` | 6000 | 4950 | 0.7883 | 0.2505 | 0.2244 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 51 | without_timing | `0f215fe8a9b82707703b29296508ed6402057a2376dce85c316f9c61016f8275` | 6000 | 4950 | 0.8187 | 0.3480 | 0.3000 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 51 | without_amount | `0f215fe8a9b82707703b29296508ed6402057a2376dce85c316f9c61016f8275` | 6000 | 4950 | 0.7934 | 0.1434 | 0.2222 | 0.0000 | 0.0654 |
| PooledPerOperatorRounds | PersonaSession | 51 | without_sequence | `0f215fe8a9b82707703b29296508ed6402057a2376dce85c316f9c61016f8275` | 6000 | 4950 | 0.8016 | 0.1212 | 0.2178 | 0.0002 | 0.1317 |
| PooledPerOperatorRounds | PersonaSession | 51 | without_synchrony | `0f215fe8a9b82707703b29296508ed6402057a2376dce85c316f9c61016f8275` | 6000 | 4950 | 0.7931 | 0.2486 | 0.2333 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 51 | without_funding | `0f215fe8a9b82707703b29296508ed6402057a2376dce85c316f9c61016f8275` | 6000 | 4950 | 0.7632 | 0.2622 | 0.1333 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 71 | none | `9581b1eee641bef7f8ab54e71476f65c578b88b1adb0f0ec458d1e208a31d8fa` | 6000 | 4950 | 0.8027 | 0.2345 | 0.2089 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 71 | without_timing | `9581b1eee641bef7f8ab54e71476f65c578b88b1adb0f0ec458d1e208a31d8fa` | 6000 | 4950 | 0.8402 | 0.3874 | 0.3333 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 71 | without_amount | `9581b1eee641bef7f8ab54e71476f65c578b88b1adb0f0ec458d1e208a31d8fa` | 6000 | 4950 | 0.8044 | 0.1540 | 0.2022 | -0.0002 | 0.0946 |
| PooledPerOperatorRounds | PersonaSession | 71 | without_sequence | `9581b1eee641bef7f8ab54e71476f65c578b88b1adb0f0ec458d1e208a31d8fa` | 6000 | 4950 | 0.8141 | 0.1335 | 0.2156 | 0.0006 | 0.1201 |
| PooledPerOperatorRounds | PersonaSession | 71 | without_synchrony | `9581b1eee641bef7f8ab54e71476f65c578b88b1adb0f0ec458d1e208a31d8fa` | 6000 | 4950 | 0.8076 | 0.2556 | 0.2200 | 0.0000 | 0.0000 |
| PooledPerOperatorRounds | PersonaSession | 71 | without_funding | `9581b1eee641bef7f8ab54e71476f65c578b88b1adb0f0ec458d1e208a31d8fa` | 6000 | 4950 | 0.7465 | 0.2464 | 0.1289 | 0.0000 | 0.0000 |

## Per-Feature Separation

Feature separation is the mean score for same-controller pairs minus the mean score for different-controller pairs.

| Funding scheme | Planner | Seed | Feature | Within mean | Between mean | Separation | ROC AUC |
|---|---|---:|---|---:|---:|---:|---:|
| DedicatedPerOperator | NaiveUniform | 11 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| DedicatedPerOperator | NaiveUniform | 11 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| DedicatedPerOperator | NaiveUniform | 11 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | FundingRound | 0.2097 | 0.0000 | 0.2097 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | FundingBatch | 0.1209 | -0.0000 | 0.1209 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 11 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| DedicatedPerOperator | NaiveUniform | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | NaiveUniform | 23 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| DedicatedPerOperator | NaiveUniform | 23 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| DedicatedPerOperator | NaiveUniform | 23 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | FundingRound | 0.2088 | 0.0000 | 0.2088 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | FundingBatch | 0.1236 | -0.0000 | 0.1236 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 23 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| DedicatedPerOperator | NaiveUniform | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | NaiveUniform | 37 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| DedicatedPerOperator | NaiveUniform | 37 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| DedicatedPerOperator | NaiveUniform | 37 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | FundingRound | 0.2072 | 0.0000 | 0.2072 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | FundingBatch | 0.1085 | -0.0000 | 0.1085 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 37 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| DedicatedPerOperator | NaiveUniform | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | NaiveUniform | 51 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| DedicatedPerOperator | NaiveUniform | 51 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| DedicatedPerOperator | NaiveUniform | 51 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | FundingRound | 0.2083 | 0.0000 | 0.2083 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | FundingBatch | 0.1184 | -0.0000 | 0.1184 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 51 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| DedicatedPerOperator | NaiveUniform | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | NaiveUniform | 71 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| DedicatedPerOperator | NaiveUniform | 71 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| DedicatedPerOperator | NaiveUniform | 71 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | FundingRound | 0.2019 | 0.0000 | 0.2019 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | FundingBatch | 0.1077 | -0.0000 | 0.1077 | 1.0000 |
| DedicatedPerOperator | NaiveUniform | 71 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| DedicatedPerOperator | NaiveUniform | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | IndependentWeighted | 11 | Timing | 0.8033 | 0.8040 | -0.0008 | 0.4988 |
| DedicatedPerOperator | IndependentWeighted | 11 | Amount | 0.7553 | 0.7561 | -0.0008 | 0.4892 |
| DedicatedPerOperator | IndependentWeighted | 11 | Sequence | 0.8796 | 0.8794 | 0.0002 | 0.4976 |
| DedicatedPerOperator | IndependentWeighted | 11 | Destination | 0.7380 | 0.7405 | -0.0026 | 0.4870 |
| DedicatedPerOperator | IndependentWeighted | 11 | Synchrony | 0.0043 | 0.0042 | 0.0001 | 0.5012 |
| DedicatedPerOperator | IndependentWeighted | 11 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 11 | FundingRound | 0.2097 | 0.0000 | 0.2097 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 11 | FundingBatch | 0.1209 | -0.0000 | 0.1209 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 11 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | IndependentWeighted | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | IndependentWeighted | 23 | Timing | 0.8056 | 0.8033 | 0.0024 | 0.5170 |
| DedicatedPerOperator | IndependentWeighted | 23 | Amount | 0.7609 | 0.7606 | 0.0002 | 0.5030 |
| DedicatedPerOperator | IndependentWeighted | 23 | Sequence | 0.8660 | 0.8670 | -0.0009 | 0.4905 |
| DedicatedPerOperator | IndependentWeighted | 23 | Destination | 0.7414 | 0.7427 | -0.0013 | 0.4913 |
| DedicatedPerOperator | IndependentWeighted | 23 | Synchrony | 0.0037 | 0.0041 | -0.0003 | 0.4925 |
| DedicatedPerOperator | IndependentWeighted | 23 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | FundingRound | 0.2088 | 0.0000 | 0.2088 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | FundingBatch | 0.1236 | -0.0000 | 0.1236 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 23 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | IndependentWeighted | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | IndependentWeighted | 37 | Timing | 0.7916 | 0.7909 | 0.0008 | 0.5009 |
| DedicatedPerOperator | IndependentWeighted | 37 | Amount | 0.7582 | 0.7572 | 0.0010 | 0.5057 |
| DedicatedPerOperator | IndependentWeighted | 37 | Sequence | 0.8689 | 0.8661 | 0.0028 | 0.5176 |
| DedicatedPerOperator | IndependentWeighted | 37 | Destination | 0.7443 | 0.7410 | 0.0033 | 0.5105 |
| DedicatedPerOperator | IndependentWeighted | 37 | Synchrony | 0.0043 | 0.0041 | 0.0002 | 0.5075 |
| DedicatedPerOperator | IndependentWeighted | 37 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | FundingRound | 0.2072 | 0.0000 | 0.2072 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | FundingBatch | 0.1085 | -0.0000 | 0.1085 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 37 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | IndependentWeighted | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | IndependentWeighted | 51 | Timing | 0.7954 | 0.7985 | -0.0030 | 0.4751 |
| DedicatedPerOperator | IndependentWeighted | 51 | Amount | 0.7579 | 0.7574 | 0.0005 | 0.5039 |
| DedicatedPerOperator | IndependentWeighted | 51 | Sequence | 0.8641 | 0.8632 | 0.0009 | 0.5078 |
| DedicatedPerOperator | IndependentWeighted | 51 | Destination | 0.7477 | 0.7449 | 0.0028 | 0.5108 |
| DedicatedPerOperator | IndependentWeighted | 51 | Synchrony | 0.0046 | 0.0042 | 0.0003 | 0.5072 |
| DedicatedPerOperator | IndependentWeighted | 51 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | FundingRound | 0.2083 | 0.0000 | 0.2083 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | FundingBatch | 0.1184 | -0.0000 | 0.1184 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 51 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | IndependentWeighted | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | IndependentWeighted | 71 | Timing | 0.8054 | 0.8025 | 0.0030 | 0.5155 |
| DedicatedPerOperator | IndependentWeighted | 71 | Amount | 0.7596 | 0.7567 | 0.0029 | 0.5292 |
| DedicatedPerOperator | IndependentWeighted | 71 | Sequence | 0.8707 | 0.8703 | 0.0004 | 0.5046 |
| DedicatedPerOperator | IndependentWeighted | 71 | Destination | 0.7460 | 0.7432 | 0.0029 | 0.5146 |
| DedicatedPerOperator | IndependentWeighted | 71 | Synchrony | 0.0041 | 0.0043 | -0.0001 | 0.4942 |
| DedicatedPerOperator | IndependentWeighted | 71 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 71 | FundingRound | 0.2019 | 0.0000 | 0.2019 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 71 | FundingBatch | 0.1077 | -0.0000 | 0.1077 | 1.0000 |
| DedicatedPerOperator | IndependentWeighted | 71 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | IndependentWeighted | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | PersonaSession | 11 | Timing | 0.3505 | 0.3482 | 0.0023 | 0.4999 |
| DedicatedPerOperator | PersonaSession | 11 | Amount | 0.7416 | 0.7421 | -0.0005 | 0.4983 |
| DedicatedPerOperator | PersonaSession | 11 | Sequence | 0.8043 | 0.8095 | -0.0052 | 0.4747 |
| DedicatedPerOperator | PersonaSession | 11 | Destination | 0.4396 | 0.4398 | -0.0001 | 0.4977 |
| DedicatedPerOperator | PersonaSession | 11 | Synchrony | 0.0127 | 0.0200 | -0.0073 | 0.4919 |
| DedicatedPerOperator | PersonaSession | 11 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 11 | FundingRound | 0.2097 | 0.0000 | 0.2097 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 11 | FundingBatch | 0.1209 | -0.0000 | 0.1209 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 11 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | PersonaSession | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | PersonaSession | 23 | Timing | 0.3517 | 0.3505 | 0.0011 | 0.4968 |
| DedicatedPerOperator | PersonaSession | 23 | Amount | 0.7488 | 0.7500 | -0.0012 | 0.4888 |
| DedicatedPerOperator | PersonaSession | 23 | Sequence | 0.7933 | 0.7943 | -0.0010 | 0.4986 |
| DedicatedPerOperator | PersonaSession | 23 | Destination | 0.4409 | 0.4432 | -0.0023 | 0.4849 |
| DedicatedPerOperator | PersonaSession | 23 | Synchrony | 0.0190 | 0.0204 | -0.0014 | 0.5052 |
| DedicatedPerOperator | PersonaSession | 23 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 23 | FundingRound | 0.2088 | 0.0000 | 0.2088 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 23 | FundingBatch | 0.1236 | -0.0000 | 0.1236 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 23 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | PersonaSession | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | PersonaSession | 37 | Timing | 0.3523 | 0.3514 | 0.0009 | 0.4887 |
| DedicatedPerOperator | PersonaSession | 37 | Amount | 0.7402 | 0.7409 | -0.0007 | 0.4907 |
| DedicatedPerOperator | PersonaSession | 37 | Sequence | 0.7969 | 0.7978 | -0.0009 | 0.4941 |
| DedicatedPerOperator | PersonaSession | 37 | Destination | 0.4389 | 0.4408 | -0.0019 | 0.4891 |
| DedicatedPerOperator | PersonaSession | 37 | Synchrony | 0.0210 | 0.0151 | 0.0059 | 0.5069 |
| DedicatedPerOperator | PersonaSession | 37 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 37 | FundingRound | 0.2072 | 0.0000 | 0.2072 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 37 | FundingBatch | 0.1085 | -0.0000 | 0.1085 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 37 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | PersonaSession | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | PersonaSession | 51 | Timing | 0.3479 | 0.3471 | 0.0008 | 0.5249 |
| DedicatedPerOperator | PersonaSession | 51 | Amount | 0.7440 | 0.7454 | -0.0014 | 0.4938 |
| DedicatedPerOperator | PersonaSession | 51 | Sequence | 0.8109 | 0.8132 | -0.0022 | 0.4893 |
| DedicatedPerOperator | PersonaSession | 51 | Destination | 0.4283 | 0.4283 | -0.0000 | 0.5002 |
| DedicatedPerOperator | PersonaSession | 51 | Synchrony | 0.0374 | 0.0240 | 0.0134 | 0.5066 |
| DedicatedPerOperator | PersonaSession | 51 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 51 | FundingRound | 0.2083 | 0.0000 | 0.2083 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 51 | FundingBatch | 0.1184 | -0.0000 | 0.1184 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 51 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | PersonaSession | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| DedicatedPerOperator | PersonaSession | 71 | Timing | 0.3573 | 0.3548 | 0.0025 | 0.5033 |
| DedicatedPerOperator | PersonaSession | 71 | Amount | 0.7440 | 0.7439 | 0.0002 | 0.5086 |
| DedicatedPerOperator | PersonaSession | 71 | Sequence | 0.7842 | 0.7875 | -0.0033 | 0.4940 |
| DedicatedPerOperator | PersonaSession | 71 | Destination | 0.4408 | 0.4424 | -0.0016 | 0.4907 |
| DedicatedPerOperator | PersonaSession | 71 | Synchrony | 0.0224 | 0.0213 | 0.0010 | 0.4993 |
| DedicatedPerOperator | PersonaSession | 71 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 71 | FundingRound | 0.2019 | 0.0000 | 0.2019 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 71 | FundingBatch | 0.1077 | -0.0000 | 0.1077 | 1.0000 |
| DedicatedPerOperator | PersonaSession | 71 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| DedicatedPerOperator | PersonaSession | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | NaiveUniform | 11 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledMixedRounds | NaiveUniform | 11 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 11 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledMixedRounds | NaiveUniform | 11 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 11 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 11 | Funding | 0.2901 | 0.2926 | -0.0025 | 0.5005 |
| PooledMixedRounds | NaiveUniform | 11 | FundingRound | 0.0166 | 0.0191 | -0.0025 | 0.4908 |
| PooledMixedRounds | NaiveUniform | 11 | FundingBatch | 0.0066 | 0.0073 | -0.0007 | 0.4916 |
| PooledMixedRounds | NaiveUniform | 11 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledMixedRounds | NaiveUniform | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | NaiveUniform | 23 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledMixedRounds | NaiveUniform | 23 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 23 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledMixedRounds | NaiveUniform | 23 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 23 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 23 | Funding | 0.2912 | 0.2831 | 0.0081 | 0.5131 |
| PooledMixedRounds | NaiveUniform | 23 | FundingRound | 0.0168 | 0.0191 | -0.0023 | 0.4935 |
| PooledMixedRounds | NaiveUniform | 23 | FundingBatch | 0.0068 | 0.0073 | -0.0005 | 0.4948 |
| PooledMixedRounds | NaiveUniform | 23 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledMixedRounds | NaiveUniform | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | NaiveUniform | 37 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledMixedRounds | NaiveUniform | 37 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 37 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledMixedRounds | NaiveUniform | 37 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 37 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 37 | Funding | 0.2865 | 0.2793 | 0.0072 | 0.5134 |
| PooledMixedRounds | NaiveUniform | 37 | FundingRound | 0.0230 | 0.0188 | 0.0042 | 0.5141 |
| PooledMixedRounds | NaiveUniform | 37 | FundingBatch | 0.0076 | 0.0075 | 0.0001 | 0.5142 |
| PooledMixedRounds | NaiveUniform | 37 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledMixedRounds | NaiveUniform | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | NaiveUniform | 51 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledMixedRounds | NaiveUniform | 51 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 51 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledMixedRounds | NaiveUniform | 51 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 51 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 51 | Funding | 0.3034 | 0.2897 | 0.0137 | 0.5171 |
| PooledMixedRounds | NaiveUniform | 51 | FundingRound | 0.0169 | 0.0191 | -0.0021 | 0.4916 |
| PooledMixedRounds | NaiveUniform | 51 | FundingBatch | 0.0059 | 0.0076 | -0.0018 | 0.4915 |
| PooledMixedRounds | NaiveUniform | 51 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledMixedRounds | NaiveUniform | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | NaiveUniform | 71 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledMixedRounds | NaiveUniform | 71 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 71 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledMixedRounds | NaiveUniform | 71 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 71 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledMixedRounds | NaiveUniform | 71 | Funding | 0.2764 | 0.2890 | -0.0126 | 0.4831 |
| PooledMixedRounds | NaiveUniform | 71 | FundingRound | 0.0188 | 0.0191 | -0.0003 | 0.4983 |
| PooledMixedRounds | NaiveUniform | 71 | FundingBatch | 0.0075 | 0.0078 | -0.0003 | 0.4977 |
| PooledMixedRounds | NaiveUniform | 71 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledMixedRounds | NaiveUniform | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | IndependentWeighted | 11 | Timing | 0.8033 | 0.8040 | -0.0008 | 0.4988 |
| PooledMixedRounds | IndependentWeighted | 11 | Amount | 0.7553 | 0.7561 | -0.0008 | 0.4892 |
| PooledMixedRounds | IndependentWeighted | 11 | Sequence | 0.8796 | 0.8794 | 0.0002 | 0.4976 |
| PooledMixedRounds | IndependentWeighted | 11 | Destination | 0.7380 | 0.7405 | -0.0026 | 0.4870 |
| PooledMixedRounds | IndependentWeighted | 11 | Synchrony | 0.0043 | 0.0042 | 0.0001 | 0.5012 |
| PooledMixedRounds | IndependentWeighted | 11 | Funding | 0.2901 | 0.2926 | -0.0025 | 0.5005 |
| PooledMixedRounds | IndependentWeighted | 11 | FundingRound | 0.0166 | 0.0191 | -0.0025 | 0.4908 |
| PooledMixedRounds | IndependentWeighted | 11 | FundingBatch | 0.0066 | 0.0073 | -0.0007 | 0.4916 |
| PooledMixedRounds | IndependentWeighted | 11 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | IndependentWeighted | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | IndependentWeighted | 23 | Timing | 0.8056 | 0.8033 | 0.0024 | 0.5170 |
| PooledMixedRounds | IndependentWeighted | 23 | Amount | 0.7609 | 0.7606 | 0.0002 | 0.5030 |
| PooledMixedRounds | IndependentWeighted | 23 | Sequence | 0.8660 | 0.8670 | -0.0009 | 0.4905 |
| PooledMixedRounds | IndependentWeighted | 23 | Destination | 0.7414 | 0.7427 | -0.0013 | 0.4913 |
| PooledMixedRounds | IndependentWeighted | 23 | Synchrony | 0.0037 | 0.0041 | -0.0003 | 0.4925 |
| PooledMixedRounds | IndependentWeighted | 23 | Funding | 0.2912 | 0.2831 | 0.0081 | 0.5131 |
| PooledMixedRounds | IndependentWeighted | 23 | FundingRound | 0.0168 | 0.0191 | -0.0023 | 0.4935 |
| PooledMixedRounds | IndependentWeighted | 23 | FundingBatch | 0.0068 | 0.0073 | -0.0005 | 0.4948 |
| PooledMixedRounds | IndependentWeighted | 23 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | IndependentWeighted | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | IndependentWeighted | 37 | Timing | 0.7916 | 0.7909 | 0.0008 | 0.5009 |
| PooledMixedRounds | IndependentWeighted | 37 | Amount | 0.7582 | 0.7572 | 0.0010 | 0.5057 |
| PooledMixedRounds | IndependentWeighted | 37 | Sequence | 0.8689 | 0.8661 | 0.0028 | 0.5176 |
| PooledMixedRounds | IndependentWeighted | 37 | Destination | 0.7443 | 0.7410 | 0.0033 | 0.5105 |
| PooledMixedRounds | IndependentWeighted | 37 | Synchrony | 0.0043 | 0.0041 | 0.0002 | 0.5075 |
| PooledMixedRounds | IndependentWeighted | 37 | Funding | 0.2865 | 0.2793 | 0.0072 | 0.5134 |
| PooledMixedRounds | IndependentWeighted | 37 | FundingRound | 0.0230 | 0.0188 | 0.0042 | 0.5141 |
| PooledMixedRounds | IndependentWeighted | 37 | FundingBatch | 0.0076 | 0.0075 | 0.0001 | 0.5142 |
| PooledMixedRounds | IndependentWeighted | 37 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | IndependentWeighted | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | IndependentWeighted | 51 | Timing | 0.7954 | 0.7985 | -0.0030 | 0.4751 |
| PooledMixedRounds | IndependentWeighted | 51 | Amount | 0.7579 | 0.7574 | 0.0005 | 0.5039 |
| PooledMixedRounds | IndependentWeighted | 51 | Sequence | 0.8641 | 0.8632 | 0.0009 | 0.5078 |
| PooledMixedRounds | IndependentWeighted | 51 | Destination | 0.7477 | 0.7449 | 0.0028 | 0.5108 |
| PooledMixedRounds | IndependentWeighted | 51 | Synchrony | 0.0046 | 0.0042 | 0.0003 | 0.5072 |
| PooledMixedRounds | IndependentWeighted | 51 | Funding | 0.3034 | 0.2897 | 0.0137 | 0.5171 |
| PooledMixedRounds | IndependentWeighted | 51 | FundingRound | 0.0169 | 0.0191 | -0.0021 | 0.4916 |
| PooledMixedRounds | IndependentWeighted | 51 | FundingBatch | 0.0059 | 0.0076 | -0.0018 | 0.4915 |
| PooledMixedRounds | IndependentWeighted | 51 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | IndependentWeighted | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | IndependentWeighted | 71 | Timing | 0.8054 | 0.8025 | 0.0030 | 0.5155 |
| PooledMixedRounds | IndependentWeighted | 71 | Amount | 0.7596 | 0.7567 | 0.0029 | 0.5292 |
| PooledMixedRounds | IndependentWeighted | 71 | Sequence | 0.8707 | 0.8703 | 0.0004 | 0.5046 |
| PooledMixedRounds | IndependentWeighted | 71 | Destination | 0.7460 | 0.7432 | 0.0029 | 0.5146 |
| PooledMixedRounds | IndependentWeighted | 71 | Synchrony | 0.0041 | 0.0043 | -0.0001 | 0.4942 |
| PooledMixedRounds | IndependentWeighted | 71 | Funding | 0.2764 | 0.2890 | -0.0126 | 0.4831 |
| PooledMixedRounds | IndependentWeighted | 71 | FundingRound | 0.0188 | 0.0191 | -0.0003 | 0.4983 |
| PooledMixedRounds | IndependentWeighted | 71 | FundingBatch | 0.0075 | 0.0078 | -0.0003 | 0.4977 |
| PooledMixedRounds | IndependentWeighted | 71 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | IndependentWeighted | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | PersonaSession | 11 | Timing | 0.3505 | 0.3482 | 0.0023 | 0.4999 |
| PooledMixedRounds | PersonaSession | 11 | Amount | 0.7416 | 0.7421 | -0.0005 | 0.4983 |
| PooledMixedRounds | PersonaSession | 11 | Sequence | 0.8043 | 0.8095 | -0.0052 | 0.4747 |
| PooledMixedRounds | PersonaSession | 11 | Destination | 0.4396 | 0.4398 | -0.0001 | 0.4977 |
| PooledMixedRounds | PersonaSession | 11 | Synchrony | 0.0127 | 0.0200 | -0.0073 | 0.4919 |
| PooledMixedRounds | PersonaSession | 11 | Funding | 0.2901 | 0.2926 | -0.0025 | 0.5005 |
| PooledMixedRounds | PersonaSession | 11 | FundingRound | 0.0166 | 0.0191 | -0.0025 | 0.4908 |
| PooledMixedRounds | PersonaSession | 11 | FundingBatch | 0.0066 | 0.0073 | -0.0007 | 0.4916 |
| PooledMixedRounds | PersonaSession | 11 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | PersonaSession | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | PersonaSession | 23 | Timing | 0.3517 | 0.3505 | 0.0011 | 0.4968 |
| PooledMixedRounds | PersonaSession | 23 | Amount | 0.7488 | 0.7500 | -0.0012 | 0.4888 |
| PooledMixedRounds | PersonaSession | 23 | Sequence | 0.7933 | 0.7943 | -0.0010 | 0.4986 |
| PooledMixedRounds | PersonaSession | 23 | Destination | 0.4409 | 0.4432 | -0.0023 | 0.4849 |
| PooledMixedRounds | PersonaSession | 23 | Synchrony | 0.0190 | 0.0204 | -0.0014 | 0.5052 |
| PooledMixedRounds | PersonaSession | 23 | Funding | 0.2912 | 0.2831 | 0.0081 | 0.5131 |
| PooledMixedRounds | PersonaSession | 23 | FundingRound | 0.0168 | 0.0191 | -0.0023 | 0.4935 |
| PooledMixedRounds | PersonaSession | 23 | FundingBatch | 0.0068 | 0.0073 | -0.0005 | 0.4948 |
| PooledMixedRounds | PersonaSession | 23 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | PersonaSession | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | PersonaSession | 37 | Timing | 0.3523 | 0.3514 | 0.0009 | 0.4887 |
| PooledMixedRounds | PersonaSession | 37 | Amount | 0.7402 | 0.7409 | -0.0007 | 0.4907 |
| PooledMixedRounds | PersonaSession | 37 | Sequence | 0.7969 | 0.7978 | -0.0009 | 0.4941 |
| PooledMixedRounds | PersonaSession | 37 | Destination | 0.4389 | 0.4408 | -0.0019 | 0.4891 |
| PooledMixedRounds | PersonaSession | 37 | Synchrony | 0.0210 | 0.0151 | 0.0059 | 0.5069 |
| PooledMixedRounds | PersonaSession | 37 | Funding | 0.2865 | 0.2793 | 0.0072 | 0.5134 |
| PooledMixedRounds | PersonaSession | 37 | FundingRound | 0.0230 | 0.0188 | 0.0042 | 0.5141 |
| PooledMixedRounds | PersonaSession | 37 | FundingBatch | 0.0076 | 0.0075 | 0.0001 | 0.5142 |
| PooledMixedRounds | PersonaSession | 37 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | PersonaSession | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | PersonaSession | 51 | Timing | 0.3479 | 0.3471 | 0.0008 | 0.5249 |
| PooledMixedRounds | PersonaSession | 51 | Amount | 0.7440 | 0.7454 | -0.0014 | 0.4938 |
| PooledMixedRounds | PersonaSession | 51 | Sequence | 0.8109 | 0.8132 | -0.0022 | 0.4893 |
| PooledMixedRounds | PersonaSession | 51 | Destination | 0.4283 | 0.4283 | -0.0000 | 0.5002 |
| PooledMixedRounds | PersonaSession | 51 | Synchrony | 0.0374 | 0.0240 | 0.0134 | 0.5066 |
| PooledMixedRounds | PersonaSession | 51 | Funding | 0.3034 | 0.2897 | 0.0137 | 0.5171 |
| PooledMixedRounds | PersonaSession | 51 | FundingRound | 0.0169 | 0.0191 | -0.0021 | 0.4916 |
| PooledMixedRounds | PersonaSession | 51 | FundingBatch | 0.0059 | 0.0076 | -0.0018 | 0.4915 |
| PooledMixedRounds | PersonaSession | 51 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | PersonaSession | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledMixedRounds | PersonaSession | 71 | Timing | 0.3573 | 0.3548 | 0.0025 | 0.5033 |
| PooledMixedRounds | PersonaSession | 71 | Amount | 0.7440 | 0.7439 | 0.0002 | 0.5086 |
| PooledMixedRounds | PersonaSession | 71 | Sequence | 0.7842 | 0.7875 | -0.0033 | 0.4940 |
| PooledMixedRounds | PersonaSession | 71 | Destination | 0.4408 | 0.4424 | -0.0016 | 0.4907 |
| PooledMixedRounds | PersonaSession | 71 | Synchrony | 0.0224 | 0.0213 | 0.0010 | 0.4993 |
| PooledMixedRounds | PersonaSession | 71 | Funding | 0.2764 | 0.2890 | -0.0126 | 0.4831 |
| PooledMixedRounds | PersonaSession | 71 | FundingRound | 0.0188 | 0.0191 | -0.0003 | 0.4983 |
| PooledMixedRounds | PersonaSession | 71 | FundingBatch | 0.0075 | 0.0078 | -0.0003 | 0.4977 |
| PooledMixedRounds | PersonaSession | 71 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledMixedRounds | PersonaSession | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | NaiveUniform | 11 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledPerOperatorRounds | NaiveUniform | 11 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 11 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledPerOperatorRounds | NaiveUniform | 11 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 11 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 11 | Funding | 0.4339 | 0.2828 | 0.1511 | 0.7126 |
| PooledPerOperatorRounds | NaiveUniform | 11 | FundingRound | 0.2097 | 0.0175 | 0.1922 | 0.9589 |
| PooledPerOperatorRounds | NaiveUniform | 11 | FundingBatch | 0.1408 | 0.0104 | 0.1304 | 0.9592 |
| PooledPerOperatorRounds | NaiveUniform | 11 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledPerOperatorRounds | NaiveUniform | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | NaiveUniform | 23 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledPerOperatorRounds | NaiveUniform | 23 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 23 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledPerOperatorRounds | NaiveUniform | 23 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 23 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 23 | Funding | 0.4383 | 0.2805 | 0.1578 | 0.6999 |
| PooledPerOperatorRounds | NaiveUniform | 23 | FundingRound | 0.2088 | 0.0273 | 0.1815 | 0.9326 |
| PooledPerOperatorRounds | NaiveUniform | 23 | FundingBatch | 0.1384 | 0.0128 | 0.1256 | 0.9442 |
| PooledPerOperatorRounds | NaiveUniform | 23 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledPerOperatorRounds | NaiveUniform | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | NaiveUniform | 37 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledPerOperatorRounds | NaiveUniform | 37 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 37 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledPerOperatorRounds | NaiveUniform | 37 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 37 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 37 | Funding | 0.4638 | 0.2862 | 0.1776 | 0.7440 |
| PooledPerOperatorRounds | NaiveUniform | 37 | FundingRound | 0.2072 | 0.0142 | 0.1930 | 0.9656 |
| PooledPerOperatorRounds | NaiveUniform | 37 | FundingBatch | 0.1362 | 0.0117 | 0.1245 | 0.9521 |
| PooledPerOperatorRounds | NaiveUniform | 37 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledPerOperatorRounds | NaiveUniform | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | NaiveUniform | 51 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledPerOperatorRounds | NaiveUniform | 51 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 51 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledPerOperatorRounds | NaiveUniform | 51 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 51 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 51 | Funding | 0.4399 | 0.2881 | 0.1518 | 0.7112 |
| PooledPerOperatorRounds | NaiveUniform | 51 | FundingRound | 0.2083 | 0.0154 | 0.1930 | 0.9629 |
| PooledPerOperatorRounds | NaiveUniform | 51 | FundingBatch | 0.1393 | 0.0110 | 0.1283 | 0.9525 |
| PooledPerOperatorRounds | NaiveUniform | 51 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledPerOperatorRounds | NaiveUniform | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | NaiveUniform | 71 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| PooledPerOperatorRounds | NaiveUniform | 71 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 71 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| PooledPerOperatorRounds | NaiveUniform | 71 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 71 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PooledPerOperatorRounds | NaiveUniform | 71 | Funding | 0.4538 | 0.2887 | 0.1651 | 0.7337 |
| PooledPerOperatorRounds | NaiveUniform | 71 | FundingRound | 0.2019 | 0.0308 | 0.1711 | 0.9235 |
| PooledPerOperatorRounds | NaiveUniform | 71 | FundingBatch | 0.1144 | 0.0131 | 0.1013 | 0.9288 |
| PooledPerOperatorRounds | NaiveUniform | 71 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| PooledPerOperatorRounds | NaiveUniform | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | Timing | 0.8033 | 0.8040 | -0.0008 | 0.4988 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | Amount | 0.7553 | 0.7561 | -0.0008 | 0.4892 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | Sequence | 0.8796 | 0.8794 | 0.0002 | 0.4976 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | Destination | 0.7380 | 0.7405 | -0.0026 | 0.4870 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | Synchrony | 0.0043 | 0.0042 | 0.0001 | 0.5012 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | Funding | 0.4339 | 0.2828 | 0.1511 | 0.7126 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | FundingRound | 0.2097 | 0.0175 | 0.1922 | 0.9589 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | FundingBatch | 0.1408 | 0.0104 | 0.1304 | 0.9592 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | IndependentWeighted | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | Timing | 0.8056 | 0.8033 | 0.0024 | 0.5170 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | Amount | 0.7609 | 0.7606 | 0.0002 | 0.5030 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | Sequence | 0.8660 | 0.8670 | -0.0009 | 0.4905 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | Destination | 0.7414 | 0.7427 | -0.0013 | 0.4913 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | Synchrony | 0.0037 | 0.0041 | -0.0003 | 0.4925 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | Funding | 0.4383 | 0.2805 | 0.1578 | 0.6999 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | FundingRound | 0.2088 | 0.0273 | 0.1815 | 0.9326 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | FundingBatch | 0.1384 | 0.0128 | 0.1256 | 0.9442 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | IndependentWeighted | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | Timing | 0.7916 | 0.7909 | 0.0008 | 0.5009 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | Amount | 0.7582 | 0.7572 | 0.0010 | 0.5057 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | Sequence | 0.8689 | 0.8661 | 0.0028 | 0.5176 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | Destination | 0.7443 | 0.7410 | 0.0033 | 0.5105 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | Synchrony | 0.0043 | 0.0041 | 0.0002 | 0.5075 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | Funding | 0.4638 | 0.2862 | 0.1776 | 0.7440 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | FundingRound | 0.2072 | 0.0142 | 0.1930 | 0.9656 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | FundingBatch | 0.1362 | 0.0117 | 0.1245 | 0.9521 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | IndependentWeighted | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | Timing | 0.7954 | 0.7985 | -0.0030 | 0.4751 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | Amount | 0.7579 | 0.7574 | 0.0005 | 0.5039 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | Sequence | 0.8641 | 0.8632 | 0.0009 | 0.5078 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | Destination | 0.7477 | 0.7449 | 0.0028 | 0.5108 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | Synchrony | 0.0046 | 0.0042 | 0.0003 | 0.5072 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | Funding | 0.4399 | 0.2881 | 0.1518 | 0.7112 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | FundingRound | 0.2083 | 0.0154 | 0.1930 | 0.9629 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | FundingBatch | 0.1393 | 0.0110 | 0.1283 | 0.9525 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | IndependentWeighted | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | Timing | 0.8054 | 0.8025 | 0.0030 | 0.5155 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | Amount | 0.7596 | 0.7567 | 0.0029 | 0.5292 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | Sequence | 0.8707 | 0.8703 | 0.0004 | 0.5046 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | Destination | 0.7460 | 0.7432 | 0.0029 | 0.5146 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | Synchrony | 0.0041 | 0.0043 | -0.0001 | 0.4942 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | Funding | 0.4538 | 0.2887 | 0.1651 | 0.7337 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | FundingRound | 0.2019 | 0.0308 | 0.1711 | 0.9235 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | FundingBatch | 0.1144 | 0.0131 | 0.1013 | 0.9288 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | IndependentWeighted | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | PersonaSession | 11 | Timing | 0.3505 | 0.3482 | 0.0023 | 0.4999 |
| PooledPerOperatorRounds | PersonaSession | 11 | Amount | 0.7416 | 0.7421 | -0.0005 | 0.4983 |
| PooledPerOperatorRounds | PersonaSession | 11 | Sequence | 0.8043 | 0.8095 | -0.0052 | 0.4747 |
| PooledPerOperatorRounds | PersonaSession | 11 | Destination | 0.4396 | 0.4398 | -0.0001 | 0.4977 |
| PooledPerOperatorRounds | PersonaSession | 11 | Synchrony | 0.0127 | 0.0200 | -0.0073 | 0.4919 |
| PooledPerOperatorRounds | PersonaSession | 11 | Funding | 0.4339 | 0.2828 | 0.1511 | 0.7126 |
| PooledPerOperatorRounds | PersonaSession | 11 | FundingRound | 0.2097 | 0.0175 | 0.1922 | 0.9589 |
| PooledPerOperatorRounds | PersonaSession | 11 | FundingBatch | 0.1408 | 0.0104 | 0.1304 | 0.9592 |
| PooledPerOperatorRounds | PersonaSession | 11 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | PersonaSession | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | PersonaSession | 23 | Timing | 0.3517 | 0.3505 | 0.0011 | 0.4968 |
| PooledPerOperatorRounds | PersonaSession | 23 | Amount | 0.7488 | 0.7500 | -0.0012 | 0.4888 |
| PooledPerOperatorRounds | PersonaSession | 23 | Sequence | 0.7933 | 0.7943 | -0.0010 | 0.4986 |
| PooledPerOperatorRounds | PersonaSession | 23 | Destination | 0.4409 | 0.4432 | -0.0023 | 0.4849 |
| PooledPerOperatorRounds | PersonaSession | 23 | Synchrony | 0.0190 | 0.0204 | -0.0014 | 0.5052 |
| PooledPerOperatorRounds | PersonaSession | 23 | Funding | 0.4383 | 0.2805 | 0.1578 | 0.6999 |
| PooledPerOperatorRounds | PersonaSession | 23 | FundingRound | 0.2088 | 0.0273 | 0.1815 | 0.9326 |
| PooledPerOperatorRounds | PersonaSession | 23 | FundingBatch | 0.1384 | 0.0128 | 0.1256 | 0.9442 |
| PooledPerOperatorRounds | PersonaSession | 23 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | PersonaSession | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | PersonaSession | 37 | Timing | 0.3523 | 0.3514 | 0.0009 | 0.4887 |
| PooledPerOperatorRounds | PersonaSession | 37 | Amount | 0.7402 | 0.7409 | -0.0007 | 0.4907 |
| PooledPerOperatorRounds | PersonaSession | 37 | Sequence | 0.7969 | 0.7978 | -0.0009 | 0.4941 |
| PooledPerOperatorRounds | PersonaSession | 37 | Destination | 0.4389 | 0.4408 | -0.0019 | 0.4891 |
| PooledPerOperatorRounds | PersonaSession | 37 | Synchrony | 0.0210 | 0.0151 | 0.0059 | 0.5069 |
| PooledPerOperatorRounds | PersonaSession | 37 | Funding | 0.4638 | 0.2862 | 0.1776 | 0.7440 |
| PooledPerOperatorRounds | PersonaSession | 37 | FundingRound | 0.2072 | 0.0142 | 0.1930 | 0.9656 |
| PooledPerOperatorRounds | PersonaSession | 37 | FundingBatch | 0.1362 | 0.0117 | 0.1245 | 0.9521 |
| PooledPerOperatorRounds | PersonaSession | 37 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | PersonaSession | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | PersonaSession | 51 | Timing | 0.3479 | 0.3471 | 0.0008 | 0.5249 |
| PooledPerOperatorRounds | PersonaSession | 51 | Amount | 0.7440 | 0.7454 | -0.0014 | 0.4938 |
| PooledPerOperatorRounds | PersonaSession | 51 | Sequence | 0.8109 | 0.8132 | -0.0022 | 0.4893 |
| PooledPerOperatorRounds | PersonaSession | 51 | Destination | 0.4283 | 0.4283 | -0.0000 | 0.5002 |
| PooledPerOperatorRounds | PersonaSession | 51 | Synchrony | 0.0374 | 0.0240 | 0.0134 | 0.5066 |
| PooledPerOperatorRounds | PersonaSession | 51 | Funding | 0.4399 | 0.2881 | 0.1518 | 0.7112 |
| PooledPerOperatorRounds | PersonaSession | 51 | FundingRound | 0.2083 | 0.0154 | 0.1930 | 0.9629 |
| PooledPerOperatorRounds | PersonaSession | 51 | FundingBatch | 0.1393 | 0.0110 | 0.1283 | 0.9525 |
| PooledPerOperatorRounds | PersonaSession | 51 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | PersonaSession | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PooledPerOperatorRounds | PersonaSession | 71 | Timing | 0.3573 | 0.3548 | 0.0025 | 0.5033 |
| PooledPerOperatorRounds | PersonaSession | 71 | Amount | 0.7440 | 0.7439 | 0.0002 | 0.5086 |
| PooledPerOperatorRounds | PersonaSession | 71 | Sequence | 0.7842 | 0.7875 | -0.0033 | 0.4940 |
| PooledPerOperatorRounds | PersonaSession | 71 | Destination | 0.4408 | 0.4424 | -0.0016 | 0.4907 |
| PooledPerOperatorRounds | PersonaSession | 71 | Synchrony | 0.0224 | 0.0213 | 0.0010 | 0.4993 |
| PooledPerOperatorRounds | PersonaSession | 71 | Funding | 0.4538 | 0.2887 | 0.1651 | 0.7337 |
| PooledPerOperatorRounds | PersonaSession | 71 | FundingRound | 0.2019 | 0.0308 | 0.1711 | 0.9235 |
| PooledPerOperatorRounds | PersonaSession | 71 | FundingBatch | 0.1144 | 0.0131 | 0.1013 | 0.9288 |
| PooledPerOperatorRounds | PersonaSession | 71 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PooledPerOperatorRounds | PersonaSession | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |

## Interpretation Bound

Funding provenance is measured per scheme, not assumed. The dedicated-funder scheme stays perfectly linkable under all three funding attacks, including the scheme-aware one that weights small shared batches, and pooled uniform-denomination rounds close only the disbursement-adjacency channel this evaluator observes. Deposits into the pool, custody of the pool, the fleet-level fact that every account appears in the first round, and value or count matching across the pool boundary are unmeasured here, so lower linkage scores still do not establish transaction-graph anonymity.
