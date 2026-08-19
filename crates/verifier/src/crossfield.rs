use bin_fields::crossfield as cf;
use bin_fields::scalar::{B128 as SB, F162};
use binius_ip::channel::IPVerifierChannel;

use crate::config::B128;

const LOG_PACKING: usize = 7;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("channel: {0}")]
    Channel(#[from] binius_ip::channel::Error),
    #[error("cross-field switch: {0}")]
    Switch(&'static str),
}

fn sample_f162<C: IPVerifierChannel<B128, Elem = B128>>(channel: &mut C, n: usize) -> Vec<F162> {
    let raw = channel.sample_many(2 * n);
    (0..n)
        .map(|i| {
            let a = u128::from(raw[2 * i]);
            let b = u128::from(raw[2 * i + 1]);
            F162([a as u64, (a >> 64) as u64, (b as u64) & ((1 << 34) - 1)])
        })
        .collect()
}

fn decode_f162(e: &[B128]) -> Vec<F162> {
    e.chunks_exact(2)
        .map(|c| {
            let a = u128::from(c[0]);
            let b = u128::from(c[1]);
            F162([a as u64, (a >> 64) as u64, b as u64])
        })
        .collect()
}

pub fn verify<C>(claim: B128, eval_point: &[B128], channel: &mut C) -> Result<(), Error>
where
    C: IPVerifierChannel<B128, Elem = B128>,
{
    let l = eval_point.len() - LOG_PACKING;
    let r_lo: Vec<SB> = eval_point[..LOG_PACKING]
        .iter()
        .rev()
        .map(|&x| SB(u128::from(x)))
        .collect();
    let r_hi: Vec<SB> = eval_point[LOG_PACKING..]
        .iter()
        .rev()
        .map(|&x| SB(u128::from(x)))
        .collect();

    let _sw = tracing::debug_span!("Cross-field switch verify").entered();
    let v_b = channel.recv_many(1 << LOG_PACKING)?;
    let v: Vec<SB> = v_b.iter().map(|&x| SB(u128::from(x))).collect();

    let r_prime = sample_f162(channel, LOG_PACKING);
    let batch = cf::eq_expand_f162(&r_prime);

    let mut vf = cf::SwitchVerifier::start(&v, SB(u128::from(claim)), &r_lo, &batch)
        .map_err(Error::Switch)?;

    let mut r_pp = Vec::with_capacity(l);
    for _ in 0..l {
        let m = decode_f162(&channel.recv_many(4)?);
        let r = sample_f162(channel, 1)[0];
        vf.round([m[0], m[1]], r);
        r_pp.push(r);
    }

    let d = tracing::debug_span!("Cross-field transparent coeff")
        .in_scope(|| cf::transparent_coeff(&r_hi, &r_pp, &batch));
    drop(_sw);

    let opened = tracing::debug_span!("Explicit F162 opening (uncounted)").in_scope(|| {
        let trace_b = channel.recv_many(1 << l).expect("trace");
        let trace: Vec<SB> = trace_b.iter().map(|&x| SB(u128::from(x))).collect();
        cf::eval_pi1(&trace, &r_pp)
    });

    if vf.s != d * opened {
        return Err(Error::Switch("sumcheck final check failed"));
    }
    Ok(())
}
