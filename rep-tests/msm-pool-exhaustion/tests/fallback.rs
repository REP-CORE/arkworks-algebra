//! Fill this process's thread budget until `spawn` fails (EAGAIN), leave no free slot, then run
//! MSMs that reach `msm_bigint_wnaf`. Without the fallback the per-chunk pool build panics; with
//! it every result equals an independent reference and the fallback counter moves.
//! One test only: it changes process-wide state (threads, the global rayon pool).
use ark_bn254::{Fr, G1Affine, G1Projective};
use ark_ec::{scalar_mul::variable_base::msm_pool_fallbacks, CurveGroup, VariableBaseMSM};
use ark_std::{UniformRand, Zero};
use std::sync::mpsc;

#[test]
fn msm_survives_thread_exhaustion_and_stays_correct() {
    rayon::ThreadPoolBuilder::new().num_threads(4).build_global().unwrap();
    let mut rng = ark_std::test_rng();
    let n = 1 << 10;
    let bases: Vec<G1Affine> = (0..n).map(|_| G1Affine::rand(&mut rng)).collect();
    let scalars: Vec<Fr> = (0..n).map(|_| Fr::rand(&mut rng)).collect(); // full width: the wNAF path
    let mut reference = G1Projective::zero();
    for (b, s) in bases.iter().zip(&scalars) { reference += *b * s; }
    let reference = reference.into_affine();
    assert_eq!(G1Projective::msm(&bases, &scalars).unwrap().into_affine(), reference, "control");

    let mut held = Vec::new();
    let err = loop {
        let (tx, rx) = mpsc::channel::<()>();
        match std::thread::Builder::new().stack_size(64 * 1024).spawn(move || { let _ = rx.recv(); }) {
            Ok(h) => held.push((tx, h)),
            Err(e) => break e,
        }
    };
    assert!(held.len() > 100, "the budget filled ({} threads, {err})", held.len());
    let before = msm_pool_fallbacks();
    for _ in 0..3 {
        let v = std::panic::catch_unwind(|| G1Projective::msm(&bases, &scalars).unwrap().into_affine())
            .expect("an MSM with no free thread slot must not panic");
        assert_eq!(v, reference, "fallback result equals the reference");
        let s = std::panic::catch_unwind(|| G1Projective::msm_serial(&bases, &scalars).unwrap().into_affine())
            .expect("msm_serial reaches the pool too and must not panic");
        assert_eq!(s, reference);
    }
    assert!(msm_pool_fallbacks() > before, "the fallback path was taken");
    for (tx, h) in held { drop(tx); let _ = h.join(); }
}
