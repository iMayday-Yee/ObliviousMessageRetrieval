use crate::pvw::{PvwCiphertext, PvwParameters};
use bfv::{
    BfvParameters, Ciphertext, Encoding, GaloisKey, Modulus, Plaintext, Poly, RelinearizationKey,
    Representation, SecretKey,
};
use itertools::izip;
use ndarray::{azip, Array2, IntoNdProducer};
use rand::thread_rng;
use std::{hint, sync::Arc, task::Poll};

pub fn mul_u128_vec(a: &[u64], b: &[u64]) -> Vec<u64> {
    todo!()
}

pub fn fma_reverse_u128_vec(a: &mut [u128], b: &[u64], c: &[u64]) {
    izip!(a.iter_mut(), b.iter(), c.iter()).for_each(|(a0, b0, c0)| {
        *a0 += *b0 as u128 * *c0 as u128;
    });
}

pub fn fma_reverse_u128_poly(d: &mut Array2<u128>, s: &Poly, h: &Poly) {
    debug_assert!(s.representation == h.representation);
    debug_assert!(s.representation == Representation::Evaluation);
    azip!(
        d.outer_iter_mut(),
        s.coefficients.outer_iter(),
        h.coefficients.outer_iter()
    )
    .for_each(|mut d, a, b| {
        fma_reverse_u128_vec(
            d.as_slice_mut().unwrap(),
            a.as_slice().unwrap(),
            b.as_slice().unwrap(),
        );
    });
}

pub fn optimised_fma(s: &Ciphertext, hint_pts: &[Plaintext], sec_len: usize) -> Ciphertext {
    // only works and sec_len <= 512 otherwise overflows
    debug_assert!(sec_len <= 512);

    let ctx = s.c_ref()[0].context.clone();
    // let mut d = Poly::zero(&ctx, &Representation::Evaluation);
    let mut d_u128 = ndarray::Array2::<u128>::zeros((ctx.moduli.len(), ctx.degree));
    let mut d1_u128 = ndarray::Array2::<u128>::zeros((ctx.moduli.len(), ctx.degree));
    for i in 0..sec_len {
        // dbg!(i);
        fma_reverse_u128_poly(&mut d_u128, &s.c_ref()[0], hint_pts[i].poly_ntt_ref());
        fma_reverse_u128_poly(&mut d1_u128, &s.c_ref()[1], hint_pts[i].poly_ntt_ref());
    }

    let mut d = ndarray::Array2::<u64>::zeros((ctx.moduli.len(), ctx.degree));
    let mut d1 = ndarray::Array2::<u64>::zeros((ctx.moduli.len(), ctx.degree));
    // TODO: combine them
    azip!(
        d.outer_iter_mut(),
        d_u128.outer_iter(),
        ctx.moduli_ops.into_producer()
    )
    .for_each(|mut a, a_u128, modqi| {
        a.as_slice_mut()
            .unwrap()
            .copy_from_slice(&modqi.barret_reduction_u128_vec(a_u128.as_slice().unwrap()));
    });
    azip!(
        d1.outer_iter_mut(),
        d1_u128.outer_iter(),
        ctx.moduli_ops.into_producer()
    )
    .for_each(|mut a, a_u128, modqi| {
        a.as_slice_mut()
            .unwrap()
            .copy_from_slice(&modqi.barret_reduction_u128_vec(a_u128.as_slice().unwrap()));
    });

    let d = Poly::new(d, &ctx, Representation::Evaluation);
    let d1 = Poly::new(d1, &ctx, Representation::Evaluation);

    Ciphertext::new(vec![d, d1], s.params(), s.level())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimised_fma_works() {
        let mut rng = thread_rng();
        let params = Arc::new(BfvParameters::default(1, 1 << 15));
        dbg!(&params.ciphertext_moduli);
        let sk = SecretKey::random(&params, &mut rng);

        let mut m = params
            .plaintext_modulus_op
            .random_vec(params.polynomial_degree, &mut rng);
        let mut m2 = params
            .plaintext_modulus_op
            .random_vec(params.polynomial_degree, &mut rng);
        let pt = Plaintext::encode(&m, &params, Encoding::simd(0));
        let pt2 = Plaintext::encode(&m2, &params, Encoding::simd(0));

        let mut ct = sk.encrypt(&pt, &mut rng);
        ct.change_representation(&Representation::Evaluation);
        let pt_vec = vec![pt2; 512];

        let now = std::time::Instant::now();
        let res_ct = optimised_fma(&ct, &pt_vec, pt_vec.len());
        println!("time optimised: {:?}", now.elapsed());

        // unoptimised fma
        let now = std::time::Instant::now();
        let mut res_ct2 = &ct * &pt_vec[0];
        pt_vec.iter().skip(1).for_each(|c| {
            res_ct2.fma_reverse_inplace(&ct, c);
        });
        println!("time un-optimised: {:?}", now.elapsed());

        let v = sk.decrypt(&res_ct).decode(Encoding::simd(0));
        let v2 = sk.decrypt(&res_ct2).decode(Encoding::simd(0));

        params.plaintext_modulus_op.mul_mod_fast_vec(&mut m, &m2);
        params
            .plaintext_modulus_op
            .scalar_mul_mod_fast_vec(&mut m, pt_vec.len() as u64);

        assert_eq!(v, m);
        assert_eq!(v, v2);
    }
}
