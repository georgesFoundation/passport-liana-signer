// psbt.rs — match a PSBT against a registered policy and determine the active
// spend branch. This is the security gate's input: we only sign what matches.

use std::collections::HashSet;

use super::bitcoin::bip32::Fingerprint;
use super::bitcoin::Psbt;
use super::{descriptor, RegisteredPolicy, Result, SpendPathKind};

/// Outcome of matching a PSBT against a registered policy.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    /// Every input's witnessScript matched a script derived from this policy.
    pub matched: bool,
    /// The spend branch the PSBT intends to use (inferred from input sequence).
    pub active_path: Option<SpendPathKind>,
    /// Relative timelock (blocks) of the active path, if it is a recovery path.
    pub active_timelock_blocks: Option<u32>,
    /// Passport owns a key on the active path AND that key is in the PSBT's
    /// bip32 derivations (i.e. we can actually contribute a signature).
    pub passport_can_sign: bool,
    pub matched_inputs: usize,
    pub total_inputs: usize,
    /// Human-readable notes for the signing-review screen / debugging.
    pub reasons: Vec<String>,
}

/// Match a PSBT against a single registered policy.
/// `gap` is how many derivation indices to check per descriptor path.
pub fn match_psbt(
    psbt: &Psbt,
    policy: &RegisteredPolicy,
    passport_fp: Fingerprint,
    gap: u32,
) -> Result<MatchResult> {
    let parsed = descriptor::import(&policy.descriptor)?;
    let singles = parsed
        .descriptor
        .clone()
        .into_single_descriptors()
        .map_err(|e| super::Error::Match(format!("multipath split: {e}")))?;

    // Precompute candidate scriptPubKeys for each path/index. Matching on the
    // spk (not the witnessScript) lets one matcher cover both P2WSH and Taproot
    // policies — Taproot inputs carry no witnessScript, only an spk.
    let mut candidates: HashSet<super::bitcoin::ScriptBuf> = HashSet::new();
    for single in &singles {
        for idx in 0..gap {
            if let Ok(def) = single.at_derivation_index(idx) {
                candidates.insert(def.script_pubkey());
            }
        }
    }

    let total_inputs = psbt.inputs.len();
    let mut matched_inputs = 0;
    let mut reasons = Vec::new();

    for (i, input) in psbt.inputs.iter().enumerate() {
        match input.witness_utxo.as_ref() {
            Some(utxo) if candidates.contains(&utxo.script_pubkey) => matched_inputs += 1,
            Some(_) => reasons.push(format!("input {i}: scriptPubKey not from this policy")),
            None => reasons.push(format!("input {i}: no witness_utxo (cannot verify)")),
        }
    }
    let matched = total_inputs > 0 && matched_inputs == total_inputs;

    if !matched {
        return Ok(MatchResult {
            matched: false,
            active_path: None,
            active_timelock_blocks: None,
            passport_can_sign: false,
            matched_inputs,
            total_inputs,
            reasons,
        });
    }

    // Infer the active branch from every input's nSequence. For a decaying
    // (multi-tier) policy, a relative-block nSequence unlocks every recovery
    // tier whose older(n) it satisfies; the deepest tier reached (largest n)
    // is the one being exercised. Mixed active branches are refused because a
    // single signing confirmation would be ambiguous.
    let input_paths: Vec<Option<u32>> = psbt
        .unsigned_tx
        .input
        .iter()
        .map(|txin| {
            let seq_blocks = relative_blocks(txin.sequence);
            deepest_unlocked_recovery(policy, seq_blocks)
        })
        .collect();
    let first_path = input_paths.first().copied().flatten();
    if input_paths.iter().any(|p| *p != first_path) {
        reasons.push("inputs use mixed primary/recovery spend paths".into());
        return Ok(MatchResult {
            matched: true,
            active_path: None,
            active_timelock_blocks: None,
            passport_can_sign: false,
            matched_inputs,
            total_inputs,
            reasons,
        });
    }

    let (active_path, active_timelock_blocks) = match first_path {
        Some(n) => {
            reasons.push(format!("nSequence unlocks recovery older({n})"));
            (SpendPathKind::Recovery, Some(n))
        }
        None => (SpendPathKind::Primary, None),
    };

    // The PSBT must reference Passport's key (segwit-v0 bip32 origins OR Taproot
    // tap key origins), and Passport must own a key on a path that is spendable
    // *right now* — the primary path always, plus any recovery tier the
    // nSequence has unlocked. A key on a not-yet-matured tier cannot sign.
    let passport_in_psbt = psbt.inputs.iter().any(|inp| {
        inp.bip32_derivation.values().any(|(fp, _)| *fp == passport_fp)
            || inp.tap_key_origins.values().any(|(_, (fp, _))| *fp == passport_fp)
    });
    let fp_str = passport_fp.to_string();
    let owns_active_key = policy.paths.iter().any(|p| {
        let active = match (active_path, p.kind) {
            (SpendPathKind::Primary, SpendPathKind::Primary) => true,
            (SpendPathKind::Recovery, SpendPathKind::Recovery) => {
                p.relative_timelock_blocks == active_timelock_blocks
            }
            _ => false,
        };
        active && p.signer_fingerprints.contains(&fp_str)
    });
    let passport_can_sign = passport_in_psbt && owns_active_key;
    if !passport_can_sign {
        reasons
            .push("Passport key is not on a currently-spendable path (or not referenced by the PSBT)".into());
    }

    Ok(MatchResult {
        matched,
        active_path: Some(active_path),
        active_timelock_blocks,
        passport_can_sign,
        matched_inputs,
        total_inputs,
        reasons,
    })
}

/// Find the registered policy a PSBT belongs to, out of many.
pub fn match_against_all<'a>(
    psbt: &Psbt,
    policies: &'a [RegisteredPolicy],
    passport_fp: Fingerprint,
    gap: u32,
) -> Result<Option<(&'a RegisteredPolicy, MatchResult)>> {
    for p in policies {
        let r = match_psbt(psbt, p, passport_fp, gap)?;
        if r.matched {
            return Ok(Some((p, r)));
        }
    }
    Ok(None)
}

/// Relative block-height from an nSequence, if it encodes one.
fn relative_blocks(seq: super::bitcoin::Sequence) -> Option<u32> {
    seq.to_relative_lock_time().and_then(|lt| match lt {
        super::bitcoin::relative::LockTime::Blocks(h) => Some(h.value() as u32),
        super::bitcoin::relative::LockTime::Time(_) => None,
    })
}

fn deepest_unlocked_recovery(policy: &RegisteredPolicy, seq_blocks: Option<u32>) -> Option<u32> {
    policy
        .paths
        .iter()
        .filter(|p| matches!(p.kind, SpendPathKind::Recovery))
        .filter_map(|p| match (seq_blocks, p.relative_timelock_blocks) {
            (Some(s), Some(n)) if s >= n => Some(n),
            _ => None,
        })
        .max()
}
