//! Durable controller cursor and crash-replay proofs.

#![forbid(unsafe_code)]

use std::{collections::BTreeMap, error::Error, sync::Arc};

use chrono::{TimeZone, Utc};
use cooker_core::{
    ActionCatalog, ActionId, AgentId, AgentSnapshot, DecisionRng, PersonaBehaviorModel,
    PersonaConfig, PersonaPreset, RunId, SessionState, StateStore, StateTransition, VirtualClock,
};
use cooker_runtime::PlannerFleet;
use cooker_store::{Store, StoreIdentity};
use tempfile::TempDir;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

struct Fixture {
    _directory: TempDir,
    store: Arc<Store>,
    clock: Arc<VirtualClock>,
    run_id: RunId,
    snapshot: AgentSnapshot,
    model: Arc<PersonaBehaviorModel>,
}

fn fixture() -> Result<Fixture, Box<dyn Error>> {
    let directory = TempDir::new()?;
    let store = Arc::new(Store::open(
        directory.path().join("planner.sqlite"),
        StoreIdentity::surfpool("planner-test")?,
    )?);
    let now = Utc
        .with_ymd_and_hms(2026, 7, 17, 15, 0, 0)
        .single()
        .ok_or("invalid fixture time")?;
    let run_id = RunId(Uuid::from_u128(101));
    let agent_id = AgentId(Uuid::from_u128(102));
    let snapshot = AgentSnapshot {
        id: agent_id,
        run_id,
        next_sequence: 0,
        next_decision_at: now,
        budget_date: now.date_naive(),
        session_state: SessionState::Active,
        last_action_at: None,
        remaining_daily_budget: 500_000_000,
        model_version: "planner-test-v1".to_owned(),
    };
    store.upsert_agent(&snapshot, now)?;

    let mut persona = PersonaConfig::preset(PersonaPreset::Trader);
    persona.daily_participation_bps = 10_000;
    persona
        .transitions
        .retain(|entry| entry.from != SessionState::Active);
    persona.transitions.push(StateTransition {
        from: SessionState::Active,
        to: SessionState::Transacting,
        weight: 1,
    });
    let model = Arc::new(PersonaBehaviorModel::new(
        [7; 32],
        "planner-test-v1",
        persona,
        ActionCatalog {
            native_destinations: vec!["destination".to_owned()],
            spl_routes: Vec::new(),
            swap_routes: Vec::new(),
            allow_stake_enter: false,
        },
        10_000,
        0,
        50,
    )?);
    Ok(Fixture {
        _directory: directory,
        store,
        clock: Arc::new(VirtualClock::new(now)),
        run_id,
        snapshot,
        model,
    })
}

#[test]
fn due_decision_is_enqueued_and_cursor_advances_once() -> TestResult {
    let Fixture {
        _directory,
        store,
        clock,
        run_id,
        snapshot,
        model,
    } = fixture()?;
    let planner = PlannerFleet::new(
        store.clone(),
        BTreeMap::from([(snapshot.id, model)]),
        clock,
        run_id,
        [7; 32],
        500_000_000,
    )?;

    let summary = planner.plan_due(10)?;
    assert_eq!(summary.decisions, 1);
    assert_eq!(summary.actions_enqueued, 1);
    assert_eq!(summary.conflicts, 0);
    let action_id = ActionId::derive(run_id, snapshot.id, 0, "planner-test-v1");
    let action = store.get_action(&action_id)?;
    assert_eq!(action.agent_id, snapshot.id);
    let advanced = store.get_agent(snapshot.id)?;
    assert_eq!(advanced.next_sequence, 1);
    assert_eq!(advanced.next_decision_at, action.scheduled_at);
    assert!(
        store
            .due_agents(run_id, clock_now(&advanced), 10)?
            .is_empty()
    );
    Ok(())
}

#[test]
fn crash_after_enqueue_replays_identical_intent_before_advancing() -> TestResult {
    let Fixture {
        _directory,
        store,
        clock,
        run_id,
        snapshot,
        model,
    } = fixture()?;
    let mut rng = DecisionRng::new(
        &[7; 32],
        snapshot.id,
        snapshot.next_sequence,
        &snapshot.model_version,
    );
    let decision = model.plan_decision(&snapshot, snapshot.next_decision_at, rng.rng())?;
    assert!(store.enqueue_action(&decision.action)?);

    let planner = PlannerFleet::new(
        store.clone(),
        BTreeMap::from([(snapshot.id, model)]),
        clock,
        run_id,
        [7; 32],
        500_000_000,
    )?;
    let summary = planner.plan_due(1)?;
    assert_eq!(summary.decisions, 1);
    assert_eq!(summary.actions_enqueued, 0);
    assert_eq!(summary.actions_replayed, 1);
    assert_eq!(store.get_action(&decision.action.id)?, decision.action);
    assert_eq!(store.get_agent(snapshot.id)?.next_sequence, 1);
    Ok(())
}

fn clock_now(snapshot: &AgentSnapshot) -> chrono::DateTime<Utc> {
    snapshot.next_decision_at - chrono::TimeDelta::nanoseconds(1)
}
