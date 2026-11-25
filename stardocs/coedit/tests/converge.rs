//! SPIKE A: randomised co-editing sessions converge. Each case is a seed: random commands on
//! two or three WordCraft sessions, random send/serve/deliver order, then everything drained;
//! every copy must equal the server's, and the `.docx` written from each must be byte-identical.
//! Cases: `COEDIT_CASES` (default 10,000 two-user, 4,000 three-user, 4,000 typing-only).
//! A failure prints its seed; replay it with `COEDIT_SEED=<seed> cargo test -p stardocs-coedit replay`.

use std::sync::OnceLock;

use proptest::prelude::*;
use stardocs_coedit::sim::{DOC_NAME, Mix, Outcome, base_docx, markers, run_scenario};

fn base() -> &'static [u8] {
    static BASE: OnceLock<Vec<u8>> = OnceLock::new();
    BASE.get_or_init(base_docx)
}

fn cases(default: u32) -> u32 {
    std::env::var("COEDIT_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn config(n: u32) -> ProptestConfig {
    ProptestConfig { cases: cases(n), failure_persistence: None, ..ProptestConfig::default() }
}

const ALL: Mix = Mix { deletes: true, objects: true };

fn check(seed: u64, n: usize, events: usize, mix: Mix) -> Result<Outcome, TestCaseError> {
    let out = run_scenario(seed, n, events, mix, base()).map_err(|e| TestCaseError::fail(format!("seed {seed}: {e}")))?;
    let want = wordcraft_engine::io::save_bytes(DOC_NAME, &out.server).map_err(TestCaseError::fail)?;
    for (i, d) in out.clients.iter().enumerate() {
        let got = wordcraft_engine::io::save_bytes(DOC_NAME, d).map_err(TestCaseError::fail)?;
        prop_assert!(got == want, "seed {}: client {}'s .docx differs from the server's", seed, i);
    }
    Ok(out)
}

proptest! {
    #![proptest_config(config(10_000))]
    #[test]
    fn two_users_converge(seed in any::<u64>(), events in 10usize..80) {
        check(seed, 2, events, ALL)?;
    }
}

proptest! {
    #![proptest_config(config(4_000))]
    #[test]
    fn three_users_converge(seed in any::<u64>(), events in 10usize..100) {
        check(seed, 3, events, ALL)?;
    }

    /// Typing, Enter, Bold/Italic and styles only (nothing deletes text): every character each
    /// person typed is in the final document.
    #[test]
    fn typing_is_never_lost(seed in any::<u64>(), events in 10usize..100) {
        let out = check(seed, 3, events, Mix { deletes: false, objects: false })?;
        for c in 0..3 {
            prop_assert_eq!(markers(&out.server, c) as isize, out.typed[c], "seed {}: client {}'s typing", seed, c);
        }
    }
}

/// Totals over a fixed set of seeds, printed for the report (`--nocapture`).
#[test]
fn step_mix_report() {
    let mut total = stardocs_coedit::sim::Stats::default();
    for seed in 0..300u64 {
        let out = run_scenario(seed, 3, 80, ALL, base()).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let s = out.stats;
        total.commands += s.commands;
        total.changes += s.changes;
        total.remote_rebases += s.remote_rebases;
        total.blocks_paras_only += s.blocks_paras_only;
        for i in 0..7 {
            total.steps[i] += s.steps[i];
        }
    }
    let dropped = stardocs_coedit::DROPPED.with(|d| d.borrow().clone());
    println!(
        "300 three-user sessions: {} commands, {} changes, {} rebases; steps splice {} format {} paraProps {} split {} join {} blocks {} (paragraphs only {}) meta {}",
        total.commands,
        total.changes,
        total.remote_rebases,
        total.steps[0],
        total.steps[1],
        total.steps[2],
        total.steps[3],
        total.steps[4],
        total.steps[5],
        total.blocks_paras_only,
        total.steps[6]
    );
    println!("late-change steps the server dropped, by (dropped step, concurrent step): {dropped:?}");
}

#[test]
fn replay() {
    if let Some(seed) = std::env::var("COEDIT_SEED").ok().and_then(|v| v.parse::<u64>().ok()) {
        let n = std::env::var("COEDIT_USERS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let events = std::env::var("COEDIT_EVENTS").ok().and_then(|v| v.parse().ok()).unwrap_or(80);
        run_scenario(seed, n, events, ALL, base()).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
    }
}
