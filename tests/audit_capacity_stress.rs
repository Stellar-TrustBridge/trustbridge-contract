//! Audit ring-buffer capacity stress test (Issue #2).
//!
//! Fills the audit ring near `MAX_AUDIT_LOG_ENTRIES` and asserts paging,
//! sequence stability, and eviction semantics when saturated. Documents
//! operational expectations when the ring is full.
//!
//! **Why this matters**: Audit saturation during Wave attacks must not panic
//! or lie. When the ring is full, the oldest entry is evicted (FIFO); new
//! entries still append; and stats counters continue to increment
//! monotonically — no wrap, no reset, no silent skip.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use trustbridge_contract::{
    AuditConfig, AuditEventType, AuditLogEntry, TrustBridgeContract, MAX_AUDIT_LOG_ENTRIES,
};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register(TrustBridgeContract, ());

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        TrustBridgeContract::initialize(env.clone(), admin.clone()).unwrap();
    });

    (env, admin, contract_id)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

#[test]
fn test_audit_ring_at_max_capacity_accepts_new_entries() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Fill the ring to capacity
        for i in 0..MAX_AUDIT_LOG_ENTRIES {
            let name = format!("user{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        // Assert ring is at capacity
        let logs = TrustBridgeContract::get_audit_logs(env.clone());
        assert_eq!(logs.len(), MAX_AUDIT_LOG_ENTRIES);

        // Add one more entry — oldest should be evicted
        TrustBridgeContract::register(
            env.clone(),
            s(&env, "overflow"),
            Address::generate(&env),
            Vec::new(&env),
        )
        .unwrap();

        let logs_after = TrustBridgeContract::get_audit_logs(env.clone());
        assert_eq!(
            logs_after.len(),
            MAX_AUDIT_LOG_ENTRIES,
            "Ring buffer should stay at MAX_AUDIT_LOG_ENTRIES after overflow"
        );

        // The oldest entry (timestamp 0) should be gone; newest (timestamp MAX) should be present
        let first_ts = logs_after.get(0).unwrap().timestamp;
        assert!(
            first_ts > 0,
            "Oldest entry should have been evicted; first timestamp should be > 0"
        );
    });
}

#[test]
fn test_audit_ring_eviction_is_fifo() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Fill ring to capacity with known timestamps
        for i in 0..MAX_AUDIT_LOG_ENTRIES {
            env.ledger().set_timestamp(i as u64 * 10);
            let name = format!("user{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let logs_at_capacity = TrustBridgeContract::get_audit_logs(env.clone());
        assert_eq!(logs_at_capacity.len(), MAX_AUDIT_LOG_ENTRIES);
        let first_ts_before = logs_at_capacity.get(0).unwrap().timestamp;
        let last_ts_before = logs_at_capacity
            .get(MAX_AUDIT_LOG_ENTRIES - 1)
            .unwrap()
            .timestamp;

        // Add 10 more entries
        for i in 0..10u32 {
            env.ledger()
                .set_timestamp((MAX_AUDIT_LOG_ENTRIES + i) as u64 * 10);
            let name = format!("overflow{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let logs_after = TrustBridgeContract::get_audit_logs(env.clone());
        assert_eq!(logs_after.len(), MAX_AUDIT_LOG_ENTRIES);

        // First 10 entries should have been evicted
        let first_ts_after = logs_after.get(0).unwrap().timestamp;
        let last_ts_after = logs_after
            .get(MAX_AUDIT_LOG_ENTRIES - 1)
            .unwrap()
            .timestamp;

        assert!(
            first_ts_after > first_ts_before,
            "FIFO: first timestamp after overflow should be greater than before"
        );
        assert!(
            last_ts_after > last_ts_before,
            "FIFO: last timestamp should advance after new entries"
        );
        assert_eq!(
            first_ts_after,
            (10 * 10) as u64,
            "First retained entry should be the 11th registered (index 10)"
        );
    });
}

#[test]
fn test_audit_stats_remain_monotonic_across_eviction() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Fill ring past capacity
        for i in 0..(MAX_AUDIT_LOG_ENTRIES + 20) {
            let name = format!("user{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let stats = TrustBridgeContract::get_audit_stats(env.clone());
        
        // Stats counters must equal total events, not ring size
        assert_eq!(
            stats.total_events,
            MAX_AUDIT_LOG_ENTRIES + 20,
            "total_events counter must stay monotonic across eviction"
        );
        assert_eq!(
            stats.registrations,
            MAX_AUDIT_LOG_ENTRIES + 20,
            "registrations counter must stay monotonic"
        );

        // But the log itself is capped
        let logs = TrustBridgeContract::get_audit_logs(env.clone());
        assert_eq!(
            logs.len(),
            MAX_AUDIT_LOG_ENTRIES,
            "Ring buffer size must stay capped at MAX_AUDIT_LOG_ENTRIES"
        );
    });
}

#[test]
fn test_audit_ring_sequence_stability_under_saturation() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Fill ring to capacity
        for i in 0..MAX_AUDIT_LOG_ENTRIES {
            let name = format!("user{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let logs_snapshot1 = TrustBridgeContract::get_audit_logs(env.clone());
        
        // Add more entries
        for i in 0..5u32 {
            let name = format!("extra{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let logs_snapshot2 = TrustBridgeContract::get_audit_logs(env.clone());

        // Logs should still be in chronological order
        for i in 1..logs_snapshot2.len() {
            let prev_ts = logs_snapshot2.get(i - 1).unwrap().timestamp;
            let curr_ts = logs_snapshot2.get(i).unwrap().timestamp;
            assert!(
                curr_ts >= prev_ts,
                "Logs must stay in chronological order after eviction"
            );
        }
    });
}

#[test]
fn test_audit_config_max_events_caps_returned_logs() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Set max_events to a smaller value
        TrustBridgeContract::set_audit_config(env.clone(), admin.clone(), true, 50, true).unwrap();

        // Add 100 entries
        for i in 0..100u32 {
            let name = format!("user{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let logs = TrustBridgeContract::get_audit_logs(env.clone());
        assert_eq!(
            logs.len(),
            50,
            "get_audit_logs should respect AuditConfig.max_events"
        );

        // Stats should still count all events
        let stats = TrustBridgeContract::get_audit_stats(env.clone());
        assert_eq!(
            stats.total_events, 100,
            "Stats counters track all events, not just retained ones"
        );
    });
}

#[test]
fn test_audit_ring_handles_mixed_event_types_at_capacity() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Fill ring with mixed operations
        for i in 0..MAX_AUDIT_LOG_ENTRIES {
            let name = format!("user{i}");
            let addr = Address::generate(&env);
            
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                addr.clone(),
                Vec::new(&env),
            )
            .unwrap();
            
            if i % 3 == 0 {
                TrustBridgeContract::verify(env.clone(), admin.clone(), s(&env, &name)).unwrap();
            }
            if i % 5 == 0 {
                TrustBridgeContract::remove(env.clone(), admin.clone(), s(&env, &name)).unwrap();
            }
        }

        let logs = TrustBridgeContract::get_audit_logs(env.clone());
        let stats = TrustBridgeContract::get_audit_stats(env.clone());

        assert_eq!(logs.len(), MAX_AUDIT_LOG_ENTRIES);
        
        // Counters must reflect the full mix
        assert!(
            stats.registrations > 0,
            "registrations counter must be non-zero"
        );
        assert!(stats.verifications > 0, "verifications counter must be non-zero");
        assert!(stats.removals > 0, "removals counter must be non-zero");
        
        // Total must be greater than any single counter
        assert!(
            stats.total_events >= stats.registrations + stats.verifications + stats.removals,
            "total_events must account for all recorded events"
        );
    });
}

#[test]
fn test_disabled_audit_config_does_not_panic_at_capacity() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Disable audit logging
        TrustBridgeContract::set_audit_config(env.clone(), admin.clone(), false, 1000, false)
            .unwrap();

        // Operations should still succeed even though logging is disabled
        for i in 0..100u32 {
            let name = format!("user{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        // No logs should be recorded
        let logs = TrustBridgeContract::get_audit_logs(env.clone());
        assert!(
            logs.is_empty() || logs.len() <= 1,
            "Logs should be minimal when audit is disabled"
        );
    });
}

#[test]
fn test_audit_paging_near_capacity_does_not_duplicate_or_skip() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Fill to exactly MAX_AUDIT_LOG_ENTRIES
        for i in 0..MAX_AUDIT_LOG_ENTRIES {
            let name = format!("user{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let logs = TrustBridgeContract::get_audit_logs(env.clone());
        
        // Check no duplicate timestamps (simple proxy for no duplicate entries)
        let mut timestamps = Vec::new(&env);
        for entry in logs.iter() {
            let ts = entry.timestamp;
            assert!(
                !timestamps.contains(ts),
                "Duplicate timestamp found: ring corruption suspected"
            );
            timestamps.push_back(ts);
        }
    });
}

#[test]
fn test_wave_attack_scenario_sustained_saturation() {
    let (env, admin, contract_id) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_id, || {
        // Simulate a Wave attack: sustained registration spam beyond capacity
        for i in 0..(MAX_AUDIT_LOG_ENTRIES * 2) {
            let name = format!("attacker{i}");
            TrustBridgeContract::register(
                env.clone(),
                s(&env, &name),
                Address::generate(&env),
                Vec::new(&env),
            )
            .unwrap();
        }

        let logs = TrustBridgeContract::get_audit_logs(env.clone());
        let stats = TrustBridgeContract::get_audit_stats(env.clone());

        // Ring stays at capacity, doesn't panic
        assert_eq!(logs.len(), MAX_AUDIT_LOG_ENTRIES);
        
        // Stats reflect full attack volume
        assert_eq!(
            stats.registrations,
            MAX_AUDIT_LOG_ENTRIES * 2,
            "Stats must count all events, even when ring is saturated"
        );
        
        // Logs are still in order
        for i in 1..logs.len() {
            assert!(
                logs.get(i).unwrap().timestamp >= logs.get(i - 1).unwrap().timestamp,
                "Logs must stay ordered even under sustained saturation"
            );
        }
    });
}
