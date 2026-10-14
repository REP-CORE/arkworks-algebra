// Deterministic repro: does arkworks msm_bigint_wnaf panic when the per-chunk rayon pool
// cannot spawn its threads (pthread_create -> EAGAIN), and does the fallback give the right answer?
use ark_bn254::{Fr, G1Affine, G1Projective};
use ark_ec::{CurveGroup, VariableBaseMSM};
use ark_std::{UniformRand, Zero};
use std::sync::mpsc;
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(1).map(String::as_str).unwrap_or("control"); // control | exhaust | bench
    let margin: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let n: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1 << 12);
    let global: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);

    rayon::ThreadPoolBuilder::new().num_threads(global).build_global().unwrap();
    let mut rng = ark_std::test_rng(); // fixed seed -> same inputs every run
    let bases: Vec<G1Affine> = (0..n).map(|_| G1Affine::rand(&mut rng)).collect();
    let kind = a.get(5).map(String::as_str).unwrap_or("full"); // full | small (<64-bit) | serial
    let scalars: Vec<Fr> = (0..n).map(|_| if kind == "small" { Fr::from(u64::rand(&mut rng) >> 1) } else { Fr::rand(&mut rng) }).collect();
    let msm = |b: &[G1Affine], s: &[Fr]| if kind == "serial" { G1Projective::msm_serial(b, s).unwrap() } else { G1Projective::msm(b, s).unwrap() };

    // Independent serial reference: plain double-and-add per term, no MSM code at all.
    let mut reference = G1Projective::zero();
    for (b, s) in bases.iter().zip(&scalars) { reference += *b * s; }
    let reference = reference.into_affine();

    if mode == "poolcost" {
        let k = 2000; let s = Instant::now();
        for _ in 0..k { let p = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap(); p.install(|| ()); drop(p); }
        println!("poolcost: build+install+drop of a 2-thread pool, mean_us={:.1}", s.elapsed().as_secs_f64()*1e6/k as f64);
        return;
    }
    if mode == "bench" {
        let k: usize = margin.max(1);
        let _ = G1Projective::msm(&bases, &scalars).unwrap();
        let mut t = vec![];
        for _ in 0..k { let s = Instant::now(); let r = G1Projective::msm(&bases, &scalars).unwrap(); t.push(s.elapsed().as_secs_f64()*1e3); assert_eq!(r.into_affine(), reference); }
        t.sort_by(|x, y| x.partial_cmp(y).unwrap());
        println!("bench n={n} global={global} runs={k} median_ms={:.2} min_ms={:.2} max_ms={:.2}", t[k/2], t[0], t[k-1]);
        return;
    }

    let mut held: Vec<(mpsc::Sender<()>, std::thread::JoinHandle<()>)> = vec![];
    if mode == "exhaust" || mode == "loop" {
        // Fill the per-process thread budget with parked threads until spawn fails.
        let err = loop {
            let (tx, rx) = mpsc::channel::<()>();
            match std::thread::Builder::new().stack_size(64 * 1024).spawn(move || { let _ = rx.recv(); }) {
                Ok(h) => held.push((tx, h)),
                Err(e) => break e,
            }
        };
        let filled = held.len();
        // Release `margin` threads (join them so they are really gone).
        for _ in 0..margin.min(held.len()) { let (tx, h) = held.pop().unwrap(); drop(tx); h.join().unwrap(); }
        eprintln!("filled={filled} spawn_error={:?} raw_os_error={:?} released={margin}", err, err.raw_os_error());
    }

    if mode == "loop" {
        // Back-to-back MSMs with `margin` free thread slots: dropped per-chunk pools exit asynchronously.
        let iters = 300; let mut fails = 0; let mut first = None;
        for i in 0..iters {
            match std::panic::catch_unwind(|| msm(&bases, &scalars).into_affine()) {
                Ok(v) => assert_eq!(v, reference),
                Err(_) => { fails += 1; if first.is_none() { first = Some(i); } }
            }
        }
        println!("mode=loop margin={margin} n={n} global={global} iters={iters} panics={fails} first_panic_iter={first:?}");
        for (tx, _) in held.drain(..) { drop(tx); }
        return;
    }
    let r = std::panic::catch_unwind(|| msm(&bases, &scalars).into_affine());
    match r {
        Ok(v) => println!("kind={kind} mode={mode} margin={margin} n={n} global={global} result=OK equals_serial_reference={}", v == reference),
        Err(p) => {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            println!("kind={kind} mode={mode} margin={margin} n={n} global={global} result=PANIC msg={msg}");
        }
    }
    for (tx, _) in held.drain(..) { drop(tx); }
}
