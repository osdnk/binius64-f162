# Cross-field switch: binius64's LIOP onto an F(2^162) PCS

Fork of [binius64](https://github.com/IrreducibleOSS/binius64) (Apache-2.0) that replaces
ring-switching + BaseFold with a cross-field switch, so the polynomial commitment runs over
`F(2^162) = F2[x]/Phi_243` instead of GHASH `F(2^128)`.

There is no embedding `F(2^128) -> F(2^162)`, so the reduction uses only the common base field
F2. The Boolean witness stays packed 128 bits per element and the whole AND/shift LIOP is
untouched; only the final claim is moved across.

## The reduction

Input is what `reduce_constraints` already produces: `w~(r) = s` with `r` in `(B128)^(7+l)` on the
bit-level witness. Write `r = (r_lo, r_hi)`, 7 and `l` coordinates.

1. Prover sends `v_i = sum_y eq(r_hi,y) w(i,y)` for the 128 bit positions. These are *exactly*
   binius64's `s_hat_v`, computed with its own `fold_1b_rows_for_b128_split`.
2. Verifier checks `s = sum_i eq(r_lo,i) v_i` — unchanged from ring-switching.
3. Verifier transposes the 128x128 bit matrix `c_{i,k} = bit k of v_i` and lifts:
   `s'_k = sum_i gamma_i c_{i,k}` in F(2^162). With `beta_i = gamma_i = X^i` the lift `phi` is
   zero-extension, so this is free.
4. Verifier samples `r'` in `(F162)^7` *after* the `v_i` are fixed and batches the 128 relations
   `s'_k = sum_y eq'(r_hi,y,k) pi1(y)` into one. Soundness of the batching: `7/|F162|`.
5. One `l`-round sumcheck over F162 of `sum_y A(y) pi1(y)`, where `A(y) = psi(eq(r_hi,y))` and
   `psi` is the F2-linear map sending `beta_k` to `eq(r',k)`.
6. Output: one evaluation claim `pi1~(r'') = z` for the F(2^162) PCS.

The verifier's coefficient `A~(r'') = sum_y eq(r'',y) psi(eq(r_hi,y))` does not factor, because
`psi` is only F2-linear. It is evaluated in `O(l)` ring operations by the same transfer-matrix
fold binius64 uses in `eval_rs_eq`, but over a *rectangular* tensor algebra
`F162 (x)_{F2} B128`: 128 F162 coefficients, `scale_vertical` a coefficient-wise F162 multiply,
`scale_horizontal` a shift-XOR by a B128 element. binius64's own `TensorAlgebra` is square and
implements `scale_horizontal` as transpose-scale-transpose, which a rectangular algebra has no
transpose for.

## What changed

```
crates/prover/src/crossfield.rs     new   prover side, spans for each stage
crates/verifier/src/crossfield.rs   new   verifier side
crates/prover/src/prove.rs          ring_switch::prove + prove_oracle_relation
                                    + finalize_oracle  ->  crossfield::prove
crates/verifier/src/verify.rs       added verify_crossfield (concrete Elem = B128)
                                    next to the generic verify; Verifier::verify calls it
crates/prover/src/ring_switch.rs    expand_tensor_factors made pub
```

The generic `verify` keeps ring-switching, which is what oracle-spec discovery, recursion and the
ZK circuit builders instantiate with symbolic element types. Oracle specs come only from
`recv_oracle`, which is unchanged, so spec discovery stays correct.

The Merkle commitment (`send_oracle`) is deliberately retained. It is not counted in the
measurements, but it keeps the trace binding, which the fake PCS below does not.

## PCS

The F(2^162) PCS is a placeholder: no commitment, the prover ships the trace, and the verifier
**explicitly evaluates** `pi1~(r'')` over F162 and checks it against the sumcheck's final claim.
So the protocol is complete and the switch is genuinely checked end-to-end; that evaluation is
excluded from the reported timings, as is the retained commitment.

Field arithmetic comes from [field-benches](https://github.com/osdnk/field-benches).

## Numbers

keccak, `--message-len 65536`, 289179 words, `l = 18`, Tiger Lake i7-11850H, median of 5.
PCS excluded on both sides: commitment, BaseFold / trace shipping, and the explicit F162 opening.

| prover | before | after |
|---|---|---|
| BitAnd check | 34.9 | 31.3 |
| shift reduction | 79.1 | 79.1 |
| ring-switch / **cross-field switch** | **4.34** | **11.24** |
| pipeline excluding PCS | 120.5 | 123.5 |
| (untimed) commitment | 13.2 | 13.0 |
| (untimed) BaseFold / ship trace | 4.31 | 5.48 |

The switch breaks down as: partial evaluations 2.19 (binius64's own routine), eq table 0.94,
transparent poly A 6.28, sumcheck 1.29, lift 0.54.

| verifier | before | after |
|---|---|---|
| total excluding PCS | 1.68 | 2.02 |
| of which the switch | — | 0.34 |
| (untimed) explicit F162 opening | — | 9.98 |

LIOP bytes: ring-switching sends 128 B128 = 2048 B. The switch sends the same 128 values plus 18
sumcheck rounds, 3200 B as encoded here (2 B128 per F162), 2804 B with tight 162-bit packing.

## Headroom

`transparent poly A` at 6.28 ms is now the switch's dominant cost: 2^18 evaluations of `psi`, each
16 byte-indexed table lookups and XORs of 24-byte values. The tables are 98 KB and miss L1.
Univariate skip does not apply to this sumcheck — it buys rounds when a multilinear's values lie
in a small *subfield*, and `pi1`'s values lie in a 128-dimensional F2-*subspace*, which is not one.
