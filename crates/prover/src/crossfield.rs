use bin_fields::crossfield as cf;
use bin_fields::scalar::{B128 as SB, F162};
use binius_field::PackedField;
use binius_ip_prover::channel::IPProverChannel;
use binius_math::FieldSlice;

use binius_verifier::config::B128;

const LOG_PACKING: usize = 7;

pub fn sample_f162<C: IPProverChannel<B128>>(channel: &mut C, n: usize) -> Vec<F162> {
    let raw = channel.sample_many(2 * n);
    (0..n)
        .map(|i| {
            let a = u128::from(raw[2 * i]);
            let b = u128::from(raw[2 * i + 1]);
            F162([a as u64, (a >> 64) as u64, (b as u64) & ((1 << 34) - 1)])
        })
        .collect()
}

pub fn encode_f162(xs: &[F162]) -> Vec<B128> {
    xs.iter()
        .flat_map(|x| {
            [
                B128::from((x.0[0] as u128) | ((x.0[1] as u128) << 64)),
                B128::from(x.0[2] as u128),
            ]
        })
        .collect()
}

pub struct CrossFieldOutput {
    pub opened: F162,
    pub r_pp: Vec<F162>,
    pub bytes_switch: usize,
}

pub fn prove<P, Channel>(
    packed_witness: FieldSlice<P>,
    eval_point: &[B128],
    channel: &mut Channel,
) -> CrossFieldOutput
where
    P: PackedField<Scalar = B128>,
    Channel: IPProverChannel<B128>,
{
    let l = eval_point.len() - LOG_PACKING;
    assert_eq!(packed_witness.log_len(), l);

    let r_hi: Vec<SB> = eval_point[LOG_PACKING..]
        .iter()
        .rev()
        .map(|&x| SB(u128::from(x)))
        .collect();
    let pi0: Vec<SB> = tracing::debug_span!("Lift packed trace")
        .in_scope(|| packed_witness.iter_scalars().map(|x| SB(u128::from(x))).collect());

    let v_b: Vec<B128> = tracing::debug_span!("Cross-field partial evaluations").in_scope(|| {
        let (eq_a, eq_b) = crate::ring_switch::expand_tensor_factors(&eval_point[LOG_PACKING..]);
        crate::ring_switch::fold_1b_rows_for_b128_split(&packed_witness, &eq_a, &eq_b)
            .iter_scalars()
            .collect()
    });
    let v: Vec<SB> = v_b.iter().map(|&x| SB(u128::from(x))).collect();
    let eq_hi = tracing::debug_span!("Cross-field eq table")
        .in_scope(|| cf::eq_expand_b128(&r_hi));
    debug_assert_eq!(v, cf::partial_evals(&pi0, &eq_hi));
    channel.send_many(&v_b);

    let r_prime = sample_f162(channel, LOG_PACKING);
    let batch = cf::eq_expand_f162(&r_prime);

    let mut pv = tracing::debug_span!("Cross-field transparent poly")
        .in_scope(|| cf::SwitchProver::new(&pi0, &eq_hi, &batch));

    let sc = tracing::debug_span!("Cross-field sumcheck").entered();
    let mut r_pp = Vec::with_capacity(l);
    for _ in 0..l {
        let m = pv.msg();
        channel.send_many(&encode_f162(&m));
        let r = sample_f162(channel, 1)[0];
        pv.fold(r);
        r_pp.push(r);
    }
    drop(sc);
    let opened = pv.final_eval();

    let bytes_switch = v_b.len() * 16 + l * 2 * 21;

    let _fake = tracing::debug_span!("Fake PCS: send trace").entered();
    let trace: Vec<B128> = packed_witness.iter_scalars().collect();
    channel.send_many(&trace);

    CrossFieldOutput {
        opened,
        r_pp,
        bytes_switch,
    }
}
