# Account Cooker Evaluation

## Experiment

- Controllers: 10
- Agents per controller: 10
- Days: 30
- Events per agent/day: 2
- Held-out seeds: [11, 23, 37, 51, 71]
- Fixed threshold: 0.550

## Composite Results

| Planner | Ablation | ROC AUC mean [95% CI] | F1 mean [95% CI] | Precision@K mean | ARI mean | NMI mean |
|---|---|---:|---:|---:|---:|---:|
| NaiveUniform | none | 1.0000 [1.0000, 1.0000] | 0.8257 [0.8257, 0.8257] | 1.0000 | 0.7179 | 0.9052 |
| NaiveUniform | without_timing | 1.0000 [1.0000, 1.0000] | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | without_amount | 1.0000 [1.0000, 1.0000] | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | without_sequence | 1.0000 [1.0000, 1.0000] | 1.0000 [1.0000, 1.0000] | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | without_synchrony | 1.0000 [1.0000, 1.0000] | 0.6000 [0.6000, 0.6000] | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | without_funding | 1.0000 [1.0000, 1.0000] | 0.6000 [0.6000, 0.6000] | 1.0000 | 0.5417 | 0.8223 |
| IndependentWeighted | none | 1.0000 [1.0000, 1.0000] | 0.1993 [0.1946, 0.2040] | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | without_timing | 1.0000 [1.0000, 1.0000] | 0.6186 [0.6037, 0.6334] | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | without_amount | 1.0000 [1.0000, 1.0000] | 0.3823 [0.3683, 0.3964] | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | without_sequence | 1.0000 [1.0000, 1.0000] | 0.6952 [0.6740, 0.7164] | 1.0000 | 0.0087 | 0.0751 |
| IndependentWeighted | without_synchrony | 1.0000 [1.0000, 1.0000] | 0.1667 [0.1667, 0.1667] | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | without_funding | 0.8035 [0.7944, 0.8126] | 0.1667 [0.1667, 0.1667] | 0.3618 | 0.0000 | 0.0000 |
| PersonaSession | none | 0.9814 [0.9808, 0.9820] | 0.7996 [0.7916, 0.8075] | 0.7791 | 0.0000 | 0.0000 |
| PersonaSession | without_timing | 0.9982 [0.9976, 0.9988] | 0.9185 [0.9080, 0.9289] | 0.9329 | 0.1231 | 0.4888 |
| PersonaSession | without_amount | 0.9817 [0.9810, 0.9825] | 0.8229 [0.8166, 0.8291] | 0.7796 | 0.0000 | 0.0131 |
| PersonaSession | without_sequence | 0.9831 [0.9827, 0.9836] | 0.8461 [0.8424, 0.8499] | 0.8093 | 0.0089 | 0.1261 |
| PersonaSession | without_synchrony | 0.9963 [0.9957, 0.9970] | 0.6099 [0.6006, 0.6192] | 0.8956 | 0.0000 | 0.0000 |
| PersonaSession | without_funding | 0.7530 [0.7466, 0.7595] | 0.2525 [0.2461, 0.2589] | 0.1413 | 0.0000 | 0.0000 |

## Per-Seed Results

| Planner | Seed | Ablation | Trace hash | Events | Pairs | ROC AUC | F1 | Precision@K | ARI | NMI |
|---|---:|---|---|---:|---:|---:|---:|---:|---:|---:|
| NaiveUniform | 11 | none | `f6dad6024ab02e51ad81c873dcf03bfe94ad01d69298d3fe4bb9d83359413e47` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| NaiveUniform | 11 | without_timing | `f6dad6024ab02e51ad81c873dcf03bfe94ad01d69298d3fe4bb9d83359413e47` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 11 | without_amount | `f6dad6024ab02e51ad81c873dcf03bfe94ad01d69298d3fe4bb9d83359413e47` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 11 | without_sequence | `f6dad6024ab02e51ad81c873dcf03bfe94ad01d69298d3fe4bb9d83359413e47` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 11 | without_synchrony | `f6dad6024ab02e51ad81c873dcf03bfe94ad01d69298d3fe4bb9d83359413e47` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 11 | without_funding | `f6dad6024ab02e51ad81c873dcf03bfe94ad01d69298d3fe4bb9d83359413e47` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 23 | none | `b058389b92f64591f63797fc15265cd637571968ec100f7fe07c945752fe42e5` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| NaiveUniform | 23 | without_timing | `b058389b92f64591f63797fc15265cd637571968ec100f7fe07c945752fe42e5` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 23 | without_amount | `b058389b92f64591f63797fc15265cd637571968ec100f7fe07c945752fe42e5` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 23 | without_sequence | `b058389b92f64591f63797fc15265cd637571968ec100f7fe07c945752fe42e5` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 23 | without_synchrony | `b058389b92f64591f63797fc15265cd637571968ec100f7fe07c945752fe42e5` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 23 | without_funding | `b058389b92f64591f63797fc15265cd637571968ec100f7fe07c945752fe42e5` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 37 | none | `a7eb77a71941cf4fb573b17aeec936ae43fc52ab8369be6ae117342ebad695f4` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| NaiveUniform | 37 | without_timing | `a7eb77a71941cf4fb573b17aeec936ae43fc52ab8369be6ae117342ebad695f4` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 37 | without_amount | `a7eb77a71941cf4fb573b17aeec936ae43fc52ab8369be6ae117342ebad695f4` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 37 | without_sequence | `a7eb77a71941cf4fb573b17aeec936ae43fc52ab8369be6ae117342ebad695f4` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 37 | without_synchrony | `a7eb77a71941cf4fb573b17aeec936ae43fc52ab8369be6ae117342ebad695f4` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 37 | without_funding | `a7eb77a71941cf4fb573b17aeec936ae43fc52ab8369be6ae117342ebad695f4` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 51 | none | `2057498b1a6e502edd8da3cea92c92997616bdefda92be7a811b57ce443abd61` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| NaiveUniform | 51 | without_timing | `2057498b1a6e502edd8da3cea92c92997616bdefda92be7a811b57ce443abd61` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 51 | without_amount | `2057498b1a6e502edd8da3cea92c92997616bdefda92be7a811b57ce443abd61` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 51 | without_sequence | `2057498b1a6e502edd8da3cea92c92997616bdefda92be7a811b57ce443abd61` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 51 | without_synchrony | `2057498b1a6e502edd8da3cea92c92997616bdefda92be7a811b57ce443abd61` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 51 | without_funding | `2057498b1a6e502edd8da3cea92c92997616bdefda92be7a811b57ce443abd61` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 71 | none | `d2fd465de9909f966858999b22d715d6bc9f2e3066d5d0e26907f1914f0293e4` | 6000 | 4950 | 1.0000 | 0.8257 | 1.0000 | 0.7179 | 0.9052 |
| NaiveUniform | 71 | without_timing | `d2fd465de9909f966858999b22d715d6bc9f2e3066d5d0e26907f1914f0293e4` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 71 | without_amount | `d2fd465de9909f966858999b22d715d6bc9f2e3066d5d0e26907f1914f0293e4` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 71 | without_sequence | `d2fd465de9909f966858999b22d715d6bc9f2e3066d5d0e26907f1914f0293e4` | 6000 | 4950 | 1.0000 | 1.0000 | 1.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 71 | without_synchrony | `d2fd465de9909f966858999b22d715d6bc9f2e3066d5d0e26907f1914f0293e4` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| NaiveUniform | 71 | without_funding | `d2fd465de9909f966858999b22d715d6bc9f2e3066d5d0e26907f1914f0293e4` | 6000 | 4950 | 1.0000 | 0.6000 | 1.0000 | 0.5417 | 0.8223 |
| IndependentWeighted | 11 | none | `bba06bcaf278bb02559ea97f487337d5dfdd77acae6741b8f24ff6ad48764687` | 6000 | 4950 | 1.0000 | 0.1933 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 11 | without_timing | `bba06bcaf278bb02559ea97f487337d5dfdd77acae6741b8f24ff6ad48764687` | 6000 | 4950 | 1.0000 | 0.6069 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 11 | without_amount | `bba06bcaf278bb02559ea97f487337d5dfdd77acae6741b8f24ff6ad48764687` | 6000 | 4950 | 1.0000 | 0.3663 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 11 | without_sequence | `bba06bcaf278bb02559ea97f487337d5dfdd77acae6741b8f24ff6ad48764687` | 6000 | 4950 | 1.0000 | 0.6844 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 11 | without_synchrony | `bba06bcaf278bb02559ea97f487337d5dfdd77acae6741b8f24ff6ad48764687` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 11 | without_funding | `bba06bcaf278bb02559ea97f487337d5dfdd77acae6741b8f24ff6ad48764687` | 6000 | 4950 | 0.7998 | 0.1667 | 0.3622 | 0.0000 | 0.0000 |
| IndependentWeighted | 23 | none | `7864769eae72030dd4a52b4a2890f0d5e4f3f28035438a9aed4bf9ec2415470c` | 6000 | 4950 | 1.0000 | 0.1995 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 23 | without_timing | `7864769eae72030dd4a52b4a2890f0d5e4f3f28035438a9aed4bf9ec2415470c` | 6000 | 4950 | 1.0000 | 0.6048 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 23 | without_amount | `7864769eae72030dd4a52b4a2890f0d5e4f3f28035438a9aed4bf9ec2415470c` | 6000 | 4950 | 1.0000 | 0.3707 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 23 | without_sequence | `7864769eae72030dd4a52b4a2890f0d5e4f3f28035438a9aed4bf9ec2415470c` | 6000 | 4950 | 1.0000 | 0.6813 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 23 | without_synchrony | `7864769eae72030dd4a52b4a2890f0d5e4f3f28035438a9aed4bf9ec2415470c` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 23 | without_funding | `7864769eae72030dd4a52b4a2890f0d5e4f3f28035438a9aed4bf9ec2415470c` | 6000 | 4950 | 0.7957 | 0.1667 | 0.3511 | 0.0000 | 0.0000 |
| IndependentWeighted | 37 | none | `a80c2b0da8dc04157ee38bfb027facab48db4698060665c80f03302211d3740e` | 6000 | 4950 | 1.0000 | 0.2059 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 37 | without_timing | `a80c2b0da8dc04157ee38bfb027facab48db4698060665c80f03302211d3740e` | 6000 | 4950 | 1.0000 | 0.6246 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 37 | without_amount | `a80c2b0da8dc04157ee38bfb027facab48db4698060665c80f03302211d3740e` | 6000 | 4950 | 1.0000 | 0.4074 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 37 | without_sequence | `a80c2b0da8dc04157ee38bfb027facab48db4698060665c80f03302211d3740e` | 6000 | 4950 | 1.0000 | 0.7365 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 37 | without_synchrony | `a80c2b0da8dc04157ee38bfb027facab48db4698060665c80f03302211d3740e` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 37 | without_funding | `a80c2b0da8dc04157ee38bfb027facab48db4698060665c80f03302211d3740e` | 6000 | 4950 | 0.8074 | 0.1667 | 0.3600 | 0.0000 | 0.0000 |
| IndependentWeighted | 51 | none | `d0ec53ab722154f3dacd14aaecb763672fea18f8918198e1d3e1ca8b697aa825` | 6000 | 4950 | 1.0000 | 0.2032 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 51 | without_timing | `d0ec53ab722154f3dacd14aaecb763672fea18f8918198e1d3e1ca8b697aa825` | 6000 | 4950 | 1.0000 | 0.6110 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 51 | without_amount | `d0ec53ab722154f3dacd14aaecb763672fea18f8918198e1d3e1ca8b697aa825` | 6000 | 4950 | 1.0000 | 0.3846 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 51 | without_sequence | `d0ec53ab722154f3dacd14aaecb763672fea18f8918198e1d3e1ca8b697aa825` | 6000 | 4950 | 1.0000 | 0.6772 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 51 | without_synchrony | `d0ec53ab722154f3dacd14aaecb763672fea18f8918198e1d3e1ca8b697aa825` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 51 | without_funding | `d0ec53ab722154f3dacd14aaecb763672fea18f8918198e1d3e1ca8b697aa825` | 6000 | 4950 | 0.7947 | 0.1667 | 0.3422 | 0.0000 | 0.0000 |
| IndependentWeighted | 71 | none | `07093805c874b892e1fc5dd5b46c80fc012c59244cf5c784009e7d50a929fd7a` | 6000 | 4950 | 1.0000 | 0.1947 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 71 | without_timing | `07093805c874b892e1fc5dd5b46c80fc012c59244cf5c784009e7d50a929fd7a` | 6000 | 4950 | 1.0000 | 0.6456 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 71 | without_amount | `07093805c874b892e1fc5dd5b46c80fc012c59244cf5c784009e7d50a929fd7a` | 6000 | 4950 | 1.0000 | 0.3825 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 71 | without_sequence | `07093805c874b892e1fc5dd5b46c80fc012c59244cf5c784009e7d50a929fd7a` | 6000 | 4950 | 1.0000 | 0.6966 | 1.0000 | 0.0435 | 0.3757 |
| IndependentWeighted | 71 | without_synchrony | `07093805c874b892e1fc5dd5b46c80fc012c59244cf5c784009e7d50a929fd7a` | 6000 | 4950 | 1.0000 | 0.1667 | 1.0000 | 0.0000 | 0.0000 |
| IndependentWeighted | 71 | without_funding | `07093805c874b892e1fc5dd5b46c80fc012c59244cf5c784009e7d50a929fd7a` | 6000 | 4950 | 0.8197 | 0.1667 | 0.3933 | 0.0000 | 0.0000 |
| PersonaSession | 11 | none | `d476e66b88ba02e8cf8a82f6b0e85bca58ce97eaa4433b40db0179e2cfb44513` | 6000 | 4950 | 0.9817 | 0.8139 | 0.7911 | 0.0000 | 0.0000 |
| PersonaSession | 11 | without_timing | `d476e66b88ba02e8cf8a82f6b0e85bca58ce97eaa4433b40db0179e2cfb44513` | 6000 | 4950 | 0.9988 | 0.9128 | 0.9578 | 0.0000 | 0.0000 |
| PersonaSession | 11 | without_amount | `d476e66b88ba02e8cf8a82f6b0e85bca58ce97eaa4433b40db0179e2cfb44513` | 6000 | 4950 | 0.9819 | 0.8303 | 0.7867 | 0.0000 | 0.0000 |
| PersonaSession | 11 | without_sequence | `d476e66b88ba02e8cf8a82f6b0e85bca58ce97eaa4433b40db0179e2cfb44513` | 6000 | 4950 | 0.9831 | 0.8520 | 0.8089 | 0.0000 | 0.0000 |
| PersonaSession | 11 | without_synchrony | `d476e66b88ba02e8cf8a82f6b0e85bca58ce97eaa4433b40db0179e2cfb44513` | 6000 | 4950 | 0.9971 | 0.6081 | 0.9000 | 0.0000 | 0.0000 |
| PersonaSession | 11 | without_funding | `d476e66b88ba02e8cf8a82f6b0e85bca58ce97eaa4433b40db0179e2cfb44513` | 6000 | 4950 | 0.7568 | 0.2442 | 0.1400 | 0.0000 | 0.0000 |
| PersonaSession | 23 | none | `77971e95a14d7354c6b8932da0e8951a9fa0b4dffbbfcf7bac81ef331d90c3af` | 6000 | 4950 | 0.9803 | 0.7986 | 0.7600 | 0.0000 | 0.0000 |
| PersonaSession | 23 | without_timing | `77971e95a14d7354c6b8932da0e8951a9fa0b4dffbbfcf7bac81ef331d90c3af` | 6000 | 4950 | 0.9985 | 0.9156 | 0.9311 | 0.0994 | 0.5268 |
| PersonaSession | 23 | without_amount | `77971e95a14d7354c6b8932da0e8951a9fa0b4dffbbfcf7bac81ef331d90c3af` | 6000 | 4950 | 0.9803 | 0.8141 | 0.7622 | 0.0000 | 0.0000 |
| PersonaSession | 23 | without_sequence | `77971e95a14d7354c6b8932da0e8951a9fa0b4dffbbfcf7bac81ef331d90c3af` | 6000 | 4950 | 0.9824 | 0.8421 | 0.7933 | 0.0000 | 0.0654 |
| PersonaSession | 23 | without_synchrony | `77971e95a14d7354c6b8932da0e8951a9fa0b4dffbbfcf7bac81ef331d90c3af` | 6000 | 4950 | 0.9955 | 0.5964 | 0.8822 | 0.0000 | 0.0000 |
| PersonaSession | 23 | without_funding | `77971e95a14d7354c6b8932da0e8951a9fa0b4dffbbfcf7bac81ef331d90c3af` | 6000 | 4950 | 0.7456 | 0.2558 | 0.1444 | 0.0000 | 0.0000 |
| PersonaSession | 37 | none | `90e9bf75cd804cbecc01eab73565ae36f5b8e1919ab9b6d05ef2a1ee8efde339` | 6000 | 4950 | 0.9821 | 0.8018 | 0.7889 | 0.0000 | 0.0000 |
| PersonaSession | 37 | without_timing | `90e9bf75cd804cbecc01eab73565ae36f5b8e1919ab9b6d05ef2a1ee8efde339` | 6000 | 4950 | 0.9986 | 0.9395 | 0.9356 | 0.1720 | 0.6391 |
| PersonaSession | 37 | without_amount | `90e9bf75cd804cbecc01eab73565ae36f5b8e1919ab9b6d05ef2a1ee8efde339` | 6000 | 4950 | 0.9826 | 0.8297 | 0.7956 | 0.0000 | 0.0000 |
| PersonaSession | 37 | without_sequence | `90e9bf75cd804cbecc01eab73565ae36f5b8e1919ab9b6d05ef2a1ee8efde339` | 6000 | 4950 | 0.9839 | 0.8493 | 0.8200 | 0.0004 | 0.0946 |
| PersonaSession | 37 | without_synchrony | `90e9bf75cd804cbecc01eab73565ae36f5b8e1919ab9b6d05ef2a1ee8efde339` | 6000 | 4950 | 0.9956 | 0.6173 | 0.8933 | 0.0000 | 0.0000 |
| PersonaSession | 37 | without_funding | `90e9bf75cd804cbecc01eab73565ae36f5b8e1919ab9b6d05ef2a1ee8efde339` | 6000 | 4950 | 0.7531 | 0.2541 | 0.1600 | 0.0000 | 0.0000 |
| PersonaSession | 51 | none | `a236796c1a8a877f43d7f166079231dd0a88993630bfb199a9858d733a2124ba` | 6000 | 4950 | 0.9812 | 0.7914 | 0.7778 | 0.0000 | 0.0000 |
| PersonaSession | 51 | without_timing | `a236796c1a8a877f43d7f166079231dd0a88993630bfb199a9858d733a2124ba` | 6000 | 4950 | 0.9970 | 0.9100 | 0.9133 | 0.1720 | 0.6391 |
| PersonaSession | 51 | without_amount | `a236796c1a8a877f43d7f166079231dd0a88993630bfb199a9858d733a2124ba` | 6000 | 4950 | 0.9817 | 0.8177 | 0.7756 | 0.0000 | 0.0654 |
| PersonaSession | 51 | without_sequence | `a236796c1a8a877f43d7f166079231dd0a88993630bfb199a9858d733a2124ba` | 6000 | 4950 | 0.9835 | 0.8444 | 0.8222 | 0.0004 | 0.0946 |
| PersonaSession | 51 | without_synchrony | `a236796c1a8a877f43d7f166079231dd0a88993630bfb199a9858d733a2124ba` | 6000 | 4950 | 0.9965 | 0.6233 | 0.9000 | 0.0000 | 0.0000 |
| PersonaSession | 51 | without_funding | `a236796c1a8a877f43d7f166079231dd0a88993630bfb199a9858d733a2124ba` | 6000 | 4950 | 0.7632 | 0.2622 | 0.1333 | 0.0000 | 0.0000 |
| PersonaSession | 71 | none | `5d428db11023847cc1bf8c14bf59a58ef00b251fd3117efca9971cc4b4cb0100` | 6000 | 4950 | 0.9818 | 0.7922 | 0.7778 | 0.0000 | 0.0000 |
| PersonaSession | 71 | without_timing | `5d428db11023847cc1bf8c14bf59a58ef00b251fd3117efca9971cc4b4cb0100` | 6000 | 4950 | 0.9981 | 0.9146 | 0.9267 | 0.1720 | 0.6391 |
| PersonaSession | 71 | without_amount | `5d428db11023847cc1bf8c14bf59a58ef00b251fd3117efca9971cc4b4cb0100` | 6000 | 4950 | 0.9821 | 0.8224 | 0.7778 | 0.0000 | 0.0000 |
| PersonaSession | 71 | without_sequence | `5d428db11023847cc1bf8c14bf59a58ef00b251fd3117efca9971cc4b4cb0100` | 6000 | 4950 | 0.9828 | 0.8429 | 0.8022 | 0.0435 | 0.3757 |
| PersonaSession | 71 | without_synchrony | `5d428db11023847cc1bf8c14bf59a58ef00b251fd3117efca9971cc4b4cb0100` | 6000 | 4950 | 0.9969 | 0.6044 | 0.9022 | 0.0000 | 0.0000 |
| PersonaSession | 71 | without_funding | `5d428db11023847cc1bf8c14bf59a58ef00b251fd3117efca9971cc4b4cb0100` | 6000 | 4950 | 0.7465 | 0.2464 | 0.1289 | 0.0000 | 0.0000 |

## Per-Feature Separation

Feature separation is the mean score for same-controller pairs minus the mean score for different-controller pairs.

| Planner | Seed | Feature | Within mean | Between mean | Separation | ROC AUC |
|---|---:|---|---:|---:|---:|---:|
| NaiveUniform | 11 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| NaiveUniform | 11 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| NaiveUniform | 11 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| NaiveUniform | 11 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 11 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 11 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 11 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| NaiveUniform | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| NaiveUniform | 23 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| NaiveUniform | 23 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| NaiveUniform | 23 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| NaiveUniform | 23 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 23 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 23 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 23 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| NaiveUniform | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| NaiveUniform | 37 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| NaiveUniform | 37 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| NaiveUniform | 37 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| NaiveUniform | 37 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 37 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 37 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 37 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| NaiveUniform | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| NaiveUniform | 51 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| NaiveUniform | 51 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| NaiveUniform | 51 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| NaiveUniform | 51 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 51 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 51 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 51 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| NaiveUniform | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| NaiveUniform | 71 | Timing | 1.0000 | 0.9271 | 0.0729 | 0.6867 |
| NaiveUniform | 71 | Amount | 1.0000 | 0.6700 | 0.3300 | 1.0000 |
| NaiveUniform | 71 | Sequence | 1.0000 | 0.1778 | 0.8222 | 0.9111 |
| NaiveUniform | 71 | Destination | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 71 | Synchrony | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 71 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| NaiveUniform | 71 | Route | 1.0000 | 0.4444 | 0.5556 | 0.7778 |
| NaiveUniform | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| IndependentWeighted | 11 | Timing | 0.8033 | 0.8040 | -0.0008 | 0.4988 |
| IndependentWeighted | 11 | Amount | 0.7553 | 0.7561 | -0.0008 | 0.4892 |
| IndependentWeighted | 11 | Sequence | 0.8796 | 0.8794 | 0.0002 | 0.4976 |
| IndependentWeighted | 11 | Destination | 0.7380 | 0.7405 | -0.0026 | 0.4870 |
| IndependentWeighted | 11 | Synchrony | 0.0043 | 0.0042 | 0.0001 | 0.5012 |
| IndependentWeighted | 11 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| IndependentWeighted | 11 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| IndependentWeighted | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| IndependentWeighted | 23 | Timing | 0.8056 | 0.8033 | 0.0024 | 0.5170 |
| IndependentWeighted | 23 | Amount | 0.7609 | 0.7606 | 0.0002 | 0.5030 |
| IndependentWeighted | 23 | Sequence | 0.8660 | 0.8670 | -0.0009 | 0.4905 |
| IndependentWeighted | 23 | Destination | 0.7414 | 0.7427 | -0.0013 | 0.4913 |
| IndependentWeighted | 23 | Synchrony | 0.0037 | 0.0041 | -0.0003 | 0.4925 |
| IndependentWeighted | 23 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| IndependentWeighted | 23 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| IndependentWeighted | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| IndependentWeighted | 37 | Timing | 0.7916 | 0.7909 | 0.0008 | 0.5009 |
| IndependentWeighted | 37 | Amount | 0.7582 | 0.7572 | 0.0010 | 0.5057 |
| IndependentWeighted | 37 | Sequence | 0.8689 | 0.8661 | 0.0028 | 0.5176 |
| IndependentWeighted | 37 | Destination | 0.7443 | 0.7410 | 0.0033 | 0.5105 |
| IndependentWeighted | 37 | Synchrony | 0.0043 | 0.0041 | 0.0002 | 0.5075 |
| IndependentWeighted | 37 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| IndependentWeighted | 37 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| IndependentWeighted | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| IndependentWeighted | 51 | Timing | 0.7954 | 0.7985 | -0.0030 | 0.4751 |
| IndependentWeighted | 51 | Amount | 0.7579 | 0.7574 | 0.0005 | 0.5039 |
| IndependentWeighted | 51 | Sequence | 0.8641 | 0.8632 | 0.0009 | 0.5078 |
| IndependentWeighted | 51 | Destination | 0.7477 | 0.7449 | 0.0028 | 0.5108 |
| IndependentWeighted | 51 | Synchrony | 0.0046 | 0.0042 | 0.0003 | 0.5072 |
| IndependentWeighted | 51 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| IndependentWeighted | 51 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| IndependentWeighted | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| IndependentWeighted | 71 | Timing | 0.8054 | 0.8025 | 0.0030 | 0.5155 |
| IndependentWeighted | 71 | Amount | 0.7596 | 0.7567 | 0.0029 | 0.5292 |
| IndependentWeighted | 71 | Sequence | 0.8707 | 0.8703 | 0.0004 | 0.5046 |
| IndependentWeighted | 71 | Destination | 0.7460 | 0.7432 | 0.0029 | 0.5146 |
| IndependentWeighted | 71 | Synchrony | 0.0041 | 0.0043 | -0.0001 | 0.4942 |
| IndependentWeighted | 71 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| IndependentWeighted | 71 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| IndependentWeighted | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PersonaSession | 11 | Timing | 0.3505 | 0.3482 | 0.0023 | 0.4999 |
| PersonaSession | 11 | Amount | 0.7416 | 0.7421 | -0.0005 | 0.4983 |
| PersonaSession | 11 | Sequence | 0.8043 | 0.8095 | -0.0052 | 0.4747 |
| PersonaSession | 11 | Destination | 0.4396 | 0.4398 | -0.0001 | 0.4977 |
| PersonaSession | 11 | Synchrony | 0.0127 | 0.0200 | -0.0073 | 0.4919 |
| PersonaSession | 11 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PersonaSession | 11 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PersonaSession | 11 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PersonaSession | 23 | Timing | 0.3517 | 0.3505 | 0.0011 | 0.4968 |
| PersonaSession | 23 | Amount | 0.7488 | 0.7500 | -0.0012 | 0.4888 |
| PersonaSession | 23 | Sequence | 0.7933 | 0.7943 | -0.0010 | 0.4986 |
| PersonaSession | 23 | Destination | 0.4409 | 0.4432 | -0.0023 | 0.4849 |
| PersonaSession | 23 | Synchrony | 0.0190 | 0.0204 | -0.0014 | 0.5052 |
| PersonaSession | 23 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PersonaSession | 23 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PersonaSession | 23 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PersonaSession | 37 | Timing | 0.3523 | 0.3514 | 0.0009 | 0.4887 |
| PersonaSession | 37 | Amount | 0.7402 | 0.7409 | -0.0007 | 0.4907 |
| PersonaSession | 37 | Sequence | 0.7969 | 0.7978 | -0.0009 | 0.4941 |
| PersonaSession | 37 | Destination | 0.4389 | 0.4408 | -0.0019 | 0.4891 |
| PersonaSession | 37 | Synchrony | 0.0210 | 0.0151 | 0.0059 | 0.5069 |
| PersonaSession | 37 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PersonaSession | 37 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PersonaSession | 37 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PersonaSession | 51 | Timing | 0.3479 | 0.3471 | 0.0008 | 0.5249 |
| PersonaSession | 51 | Amount | 0.7440 | 0.7454 | -0.0014 | 0.4938 |
| PersonaSession | 51 | Sequence | 0.8109 | 0.8132 | -0.0022 | 0.4893 |
| PersonaSession | 51 | Destination | 0.4283 | 0.4283 | -0.0000 | 0.5002 |
| PersonaSession | 51 | Synchrony | 0.0374 | 0.0240 | 0.0134 | 0.5066 |
| PersonaSession | 51 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PersonaSession | 51 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PersonaSession | 51 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |
| PersonaSession | 71 | Timing | 0.3573 | 0.3548 | 0.0025 | 0.5033 |
| PersonaSession | 71 | Amount | 0.7440 | 0.7439 | 0.0002 | 0.5086 |
| PersonaSession | 71 | Sequence | 0.7842 | 0.7875 | -0.0033 | 0.4940 |
| PersonaSession | 71 | Destination | 0.4408 | 0.4424 | -0.0016 | 0.4907 |
| PersonaSession | 71 | Synchrony | 0.0224 | 0.0213 | 0.0010 | 0.4993 |
| PersonaSession | 71 | Funding | 1.0000 | 0.0000 | 1.0000 | 1.0000 |
| PersonaSession | 71 | Route | 1.0000 | 1.0000 | 0.0000 | 0.5000 |
| PersonaSession | 71 | BalanceRank | 0.8620 | 0.4528 | 0.4092 | 0.9089 |

## Interpretation Bound

The common-funder graph remains directly observable; lower behavioral linkage scores do not establish transaction-graph anonymity.
