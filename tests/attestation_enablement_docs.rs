//! Automated proof for Issue #845: the required-attestation enablement
//! checklist must present the stage/attest flow, monitoring, and disable
//! criteria consistently across operator docs and the gating source module.

const RUNBOOK: &str = include_str!("../docs/ADMIN_RUNBOOK.md");
const ABI: &str = include_str!("../docs/ABI.md");
const DEPLOYMENT: &str = include_str!("../docs/DEPLOYMENT.md");
const STAGED: &str = include_str!("../src/staged_wasm.rs");

fn check_checklist(
    runbook: &str,
    abi: &str,
    deployment: &str,
    staged: &str,
) -> Result<(), &'static str> {
    if !runbook.contains("**Enablement checklist") {
        return Err("missing enablement checklist in docs/ADMIN_RUNBOOK.md");
    }
    if !runbook.contains("**Stage/attest flow:**")
        || !runbook.contains("`stage_wasm --wasm-hash \"$HASH\"`")
        || !runbook.contains("`attest_upgrade`")
    {
        return Err("missing stage/attest flow in checklist");
    }
    if !runbook.contains("**Monitor:**") || !runbook.contains("AttestationRequired") {
        return Err("missing monitoring guidance in checklist");
    }
    if !runbook.contains("**Disable criteria:**")
        || !runbook.contains("set_attestation_required --required false")
    {
        return Err("missing disable criteria in checklist");
    }
    if !abi.contains("Enablement checklist for operators") {
        return Err("missing checklist cross-link in docs/ABI.md");
    }
    if !deployment.contains("ADMIN_RUNBOOK enablement checklist") {
        return Err("missing checklist cross-link in docs/DEPLOYMENT.md");
    }
    if !staged.contains("operator checklist") || !staged.contains("disable criteria") {
        return Err("missing checklist cross-link in src/staged_wasm.rs");
    }
    Ok(())
}

#[test]
fn enablement_checklist_is_complete() {
    assert_eq!(check_checklist(RUNBOOK, ABI, DEPLOYMENT, STAGED), Ok(()));
}

#[test]
fn missing_stage_attest_flow_is_rejected() {
    let without = RUNBOOK.replace("**Stage/attest flow:**", "");
    assert_eq!(
        check_checklist(&without, ABI, DEPLOYMENT, STAGED),
        Err("missing stage/attest flow in checklist")
    );
}

#[test]
fn missing_disable_criteria_is_rejected() {
    let without = RUNBOOK.replace("**Disable criteria:**", "");
    assert_eq!(
        check_checklist(&without, ABI, DEPLOYMENT, STAGED),
        Err("missing disable criteria in checklist")
    );
}
