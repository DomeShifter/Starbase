//! Randomised co-editing sessions converge. Each case is a seed: random commands on two or three
//! WordCraft sessions (typing, Enter, Backspace/Delete, selections, bold/italic, styles, table
//! rows/columns/cells, comments added/replied/edited/resolved/deleted, footnotes, headers and
//! footers, tracked suggestions, and undo of one's own last change after others edited), random
//! send/serve/deliver order, then everything drained; every copy must equal the server's, and
//! the `.docx` written from each must be byte-identical.
//! Cases: `COEDIT_CASES` (default 10,000 two-user, 4,000 three-user, 4,000 typing-only).
//! A failure prints its seed; replay it with `COEDIT_SEED=<seed> cargo test -p stardocs-coedit replay`.

use std::sync::OnceLock;

use proptest::prelude::*;
use stardocs_coedit::sim::{DOC_NAME, Mix, Outcome, Stats, base_docx, markers, run_scenario};

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

const ALL: Mix = Mix { deletes: true, objects: true, undo: true };
const TYPING: Mix = Mix { deletes: false, objects: false, undo: true };

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
    fn two_users_converge(seed in any::<u64>(), events in 10usize..120) {
        check(seed, 2, events, ALL)?;
    }
}

proptest! {
    #![proptest_config(config(4_000))]
    #[test]
    fn three_users_converge(seed in any::<u64>(), events in 10usize..150) {
        check(seed, 3, events, ALL)?;
    }

    /// Typing, Enter, Bold/Italic, styles and undo only (no deletes): every character each person
    /// typed is in the final document, except what they undid themselves.
    #[test]
    fn typing_is_never_lost(seed in any::<u64>(), events in 10usize..150) {
        let out = check(seed, 3, events, TYPING)?;
        for c in 0..3 {
            prop_assert_eq!(markers(&out.server, c) as isize, out.typed[c], "seed {}: client {}'s typing", seed, c);
        }
    }
}

/// Totals over a fixed set of seeds, printed for the report (`--nocapture`).
#[test]
fn step_mix_report() {
    let mut total = Stats::default();
    for seed in 0..300u64 {
        let out = run_scenario(seed, 3, 120, ALL, base()).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let s = out.stats;
        total.commands += s.commands;
        total.changes += s.changes;
        total.remote_rebases += s.remote_rebases;
        total.undos += s.undos;
        total.undo_steps_dropped += s.undo_steps_dropped;
        for (k, v) in s.steps {
            *total.steps.entry(k).or_default() += v;
        }
    }
    println!(
        "300 three-user sessions: {} commands, {} changes, {} rebases, {} undos ({} undo steps dropped)",
        total.commands, total.changes, total.remote_rebases, total.undos, total.undo_steps_dropped
    );
    println!("steps: {:?}", total.steps);
    let dropped = stardocs_coedit::DROPPED.with(|d| d.borrow().clone());
    println!("late-change steps the server dropped, by (dropped step, concurrent step): {dropped:?}");
}

#[test]
fn replay() {
    if let Some(seed) = std::env::var("COEDIT_SEED").ok().and_then(|v| v.parse::<u64>().ok()) {
        let n = std::env::var("COEDIT_USERS").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let events = std::env::var("COEDIT_EVENTS").ok().and_then(|v| v.parse().ok()).unwrap_or(80);
        let mix = if std::env::var("COEDIT_TYPING").is_ok() { TYPING } else { ALL };
        stardocs_coedit::sim::TRACE.with(|t| t.set(std::env::var("COEDIT_TRACE").is_ok()));
        let out = run_scenario(seed, n, events, mix, base()).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        for c in 0..n {
            println!("client {c}: markers {} expected {}", markers(&out.server, c), out.typed[c]);
        }
        println!("stats {:?}", out.stats);
    }
}
