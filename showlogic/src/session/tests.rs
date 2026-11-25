use super::*;

const HOST: &str = "host-secret";
const CODE: &str = "3FA9C1";

fn policy(open: bool, code: bool, request: bool, account: bool) -> AccessPolicy {
    AccessPolicy { open, code, request, account }
}

const VIEW_CODE: &str = "LetMeIn!";

fn settings(view: AccessPolicy, caller: AccessPolicy) -> AccessSettings {
    AccessSettings {
        view,
        view_code: view.code.then(|| VIEW_CODE.to_owned()),
        caller,
        caller_code: caller.code.then(|| CODE.to_owned()),
    }
}

fn open_door() -> AccessPolicy {
    policy(true, false, false, false)
}

fn start(settings: AccessSettings) -> SessionLogic {
    SessionLogic::new("session".into(), "show".into(), HOST.into(), settings).unwrap()
}

/// Joining open, calling as given: what the caller tests are about.
fn logic(caller: AccessPolicy) -> SessionLogic {
    start(settings(open_door(), caller))
}

fn connect(logic: &mut SessionLogic, id: &str) -> Vec<Effect> {
    logic.connect(id, &format!("10.0.0.{id}"))
}

fn claim(logic: &mut SessionLogic, id: &str, claim: Claim) -> Vec<Effect> {
    logic.handle(id, ClientMessage::ClaimCaller { claim }, 100.0)
}

/// The caller status sent to `id`, with its reason and resume token.
fn status_for(effects: &[Effect], id: &str) -> Option<(AccessState, Option<String>, Option<String>)> {
    effects.iter().find_map(|effect| match effect {
        Effect::Send(to, ServerMessage::CallerStatus { state, reason, resume_token }) if to == id => {
            Some((*state, reason.clone(), resume_token.clone()))
        }
        _ => None,
    })
}

fn state_of(effects: &[Effect], id: &str) -> Option<AccessState> {
    status_for(effects, id).map(|s| s.0)
}

fn grant(logic: &mut SessionLogic, id: &str, with: Claim) -> String {
    let effects = claim(logic, id, with);
    let (state, _, token) = status_for(&effects, id).unwrap();
    assert_eq!(state, AccessState::Granted, "{effects:?}");
    token.unwrap()
}

fn open_claim(name: &str) -> Claim {
    Claim::Open { name: name.into() }
}
fn code_claim(code: &str) -> Claim {
    Claim::Code { name: "Sam".into(), code: code.into() }
}

fn cue_of(effects: &[Effect]) -> Option<(i64, Option<String>)> {
    effects.iter().find_map(|e| match e {
        Effect::Broadcast(ServerMessage::CueChange { current_cue, by, .. }) => Some((*current_cue, by.clone())),
        _ => None,
    })
}

#[test]
fn every_new_connection_is_welcomed_with_a_fresh_nonce_and_is_only_crew() {
    let mut l = logic(policy(false, true, false, false));
    let a = connect(&mut l, "1");
    let b = connect(&mut l, "2");
    let nonce = |effects: &[Effect]| match &effects[0] {
        Effect::Send(_, ServerMessage::Welcome { nonce, protocol, policy, .. }) => {
            assert_eq!(*protocol, PROTOCOL_VERSION);
            assert!(policy.code);
            nonce.clone()
        }
        other => panic!("{other:?}"),
    };
    assert_ne!(nonce(&a), nonce(&b));
    for message in [
        ClientMessage::NextCue { from_cue: None },
        ClientMessage::PreviousCue { from_cue: None },
        ClientMessage::JumpToCue { cue_number: 3 },
    ] {
        let effects = l.handle("1", message, 1.0);
        assert!(matches!(&effects[..], [Effect::Send(to, ServerMessage::Error { .. })] if to == "1"), "{effects:?}");
        assert_eq!(l.snapshot().current_cue, None, "the show did not move");
    }
}

#[test]
fn open_access_grants_at_once_and_only_when_enabled() {
    let mut l = logic(policy(true, false, false, false));
    connect(&mut l, "1");
    let effects = claim(&mut l, "1", open_claim("Sam"));
    assert_eq!(state_of(&effects, "1"), Some(AccessState::Granted));
    assert!(effects.iter().any(|e| matches!(e, Effect::Broadcast(ServerMessage::Callers { list }) if list.len() == 1 && list[0].name == "Sam")));

    let mut closed = logic(policy(false, true, true, true));
    connect(&mut closed, "1");
    assert_eq!(state_of(&claim(&mut closed, "1", open_claim("Sam")), "1"), Some(AccessState::Denied));
}

#[test]
fn a_code_is_asked_and_checked_before_granting() {
    let mut l = logic(policy(false, true, false, false));
    connect(&mut l, "1");
    let denied = claim(&mut l, "1", code_claim("000000"));
    assert_eq!(status_for(&denied, "1").unwrap().1.as_deref(), Some("Wrong code"));
    // Hex codes ignore case.
    grant(&mut l, "1", code_claim("3fa9c1"));
    // The other ways in stay closed.
    connect(&mut l, "2");
    for other in [open_claim("x"), Claim::Request { name: "x".into() }, Claim::Account { pass: "p".into(), proof: "q".into() }] {
        assert_eq!(state_of(&claim(&mut l, "2", other), "2"), Some(AccessState::Denied));
    }
}

#[test]
fn guessing_a_code_is_slowed_and_closes_the_connection() {
    let mut l = logic(policy(false, true, false, false));
    connect(&mut l, "1");
    let mut closed = false;
    let mut now = 0.0;
    for attempt in 1..=MAX_TRIES_PER_CONNECTION {
        now += 120.0; // waits out the per-address delay
        let effects = l.handle("1", ClientMessage::ClaimCaller { claim: code_claim("000000") }, now);
        closed = effects.contains(&Effect::Close("1".into()));
        assert_eq!(closed, attempt == MAX_TRIES_PER_CONNECTION, "attempt {attempt}");
    }
    assert!(closed);
    // A second connection from the same address still has to wait, even with the right code.
    l.connect("2", "10.0.0.1");
    let effects = l.handle("2", ClientMessage::ClaimCaller { claim: code_claim(CODE) }, now + 0.1);
    assert!(status_for(&effects, "2").unwrap().1.unwrap().starts_with("Too many attempts"));
    let effects = l.handle("2", ClientMessage::ClaimCaller { claim: code_claim(CODE) }, now + 120.0);
    assert_eq!(state_of(&effects, "2"), Some(AccessState::Granted));
}

#[test]
fn many_wrong_codes_pause_code_entry_and_tell_the_host() {
    let mut l = logic(policy(false, true, false, false));
    let mut told = false;
    for i in 0..20 {
        let id = format!("c{i}");
        l.connect(&id, &format!("192.168.0.{i}"));
        let effects = l.handle(&id, ClientMessage::ClaimCaller { claim: code_claim("999999") }, 1.0);
        told |= effects.contains(&Effect::HostChanged);
    }
    assert!(told);
    assert!(l.host_view(1.0).call_paused_seconds.is_some());
    // A new code starts over.
    l.set_code(AccessKind::Call, None, None).unwrap();
    assert_eq!(l.host_view(1.0).call_paused_seconds, None);
}

#[test]
fn a_request_waits_for_the_host() {
    let mut l = logic(policy(false, false, true, false));
    connect(&mut l, "1");
    let effects = claim(&mut l, "1", Claim::Request { name: "  Robin  ".into() });
    assert_eq!(state_of(&effects, "1"), Some(AccessState::Pending));
    assert_eq!(l.host_view(1.0).requests, vec![HostRequest { client_id: "1".into(), name: "Robin".into(), kind: AccessKind::Call }]);
    assert!(l.handle("1", ClientMessage::NextCue { from_cue: None }, 1.0).iter().any(|e| matches!(e, Effect::Send(_, ServerMessage::Error { .. }))));

    let approved = l.answer_request("1", AccessKind::Call, true);
    assert_eq!(state_of(&approved, "1"), Some(AccessState::Granted));
    assert!(l.host_view(1.0).requests.is_empty());
    assert_eq!(cue_of(&l.handle("1", ClientMessage::NextCue { from_cue: None }, 1.0)), Some((1, Some("Robin".into()))));

    connect(&mut l, "2");
    claim(&mut l, "2", Claim::Request { name: "Kim".into() });
    let denied = l.answer_request("2", AccessKind::Call, false);
    assert_eq!(state_of(&denied, "2"), Some(AccessState::Denied));
    assert!(l.answer_request("2", AccessKind::Call, true).is_empty(), "a request is answered once");
}

#[test]
fn every_caller_controls_and_two_presses_advance_only_once() {
    let mut l = logic(policy(true, false, false, false));
    connect(&mut l, "1");
    connect(&mut l, "2");
    grant(&mut l, "1", open_claim("Ann"));
    grant(&mut l, "2", open_claim("Ben"));
    assert_eq!(cue_of(&l.handle("1", ClientMessage::NextCue { from_cue: None }, 10.0)), Some((1, Some("Ann".into()))));
    // Ben pressed Next while still seeing "not started": ignored, and he is brought up to date.
    let stale = l.handle("2", ClientMessage::NextCue { from_cue: None }, 10.1);
    assert!(matches!(&stale[..], [Effect::Send(to, ServerMessage::CueChange { current_cue: 1, by: None, .. })] if to == "2"), "{stale:?}");
    assert_eq!(cue_of(&l.handle("2", ClientMessage::NextCue { from_cue: Some(1) }, 11.0)), Some((2, Some("Ben".into()))));
    assert_eq!(cue_of(&l.handle("1", ClientMessage::PreviousCue { from_cue: Some(2) }, 12.0)), Some((1, Some("Ann".into()))));
    assert_eq!(cue_of(&l.handle("1", ClientMessage::PreviousCue { from_cue: Some(1) }, 13.0)), Some((1, Some("Ann".into()))), "stays on the first cue");
    // A jump is explicit: it does not need the current cue.
    assert_eq!(cue_of(&l.handle("2", ClientMessage::JumpToCue { cue_number: 7 }, 14.0)), Some((7, Some("Ben".into()))));
    assert!(l.handle("2", ClientMessage::JumpToCue { cue_number: 0 }, 15.0).is_empty());
}

#[test]
fn the_show_start_is_set_once_by_the_first_next_and_each_cue_restarts_its_timer() {
    let mut l = logic(policy(true, false, false, false));
    connect(&mut l, "1");
    grant(&mut l, "1", open_claim("Ann"));
    l.handle("1", ClientMessage::JumpToCue { cue_number: 3 }, 5.0);
    assert_eq!(l.snapshot().show_started_at, None, "jumping does not start the show");
    l.handle("1", ClientMessage::NextCue { from_cue: Some(3) }, 50.0);
    assert_eq!(l.snapshot().show_started_at, Some(50.0));
    l.handle("1", ClientMessage::NextCue { from_cue: Some(4) }, 80.0);
    assert_eq!((l.snapshot().show_started_at, l.snapshot().cue_started_at), (Some(50.0), Some(80.0)));
    l.handle("1", ClientMessage::PreviousCue { from_cue: Some(5) }, 90.0);
    assert_eq!((l.snapshot().show_started_at, l.snapshot().cue_started_at), (Some(50.0), Some(90.0)));
}

#[test]
fn a_resume_token_survives_a_reconnect_until_access_ends() {
    let mut l = logic(policy(false, true, false, false));
    connect(&mut l, "1");
    let token = grant(&mut l, "1", code_claim(CODE));
    l.disconnect("1");
    assert!(l.callers().is_empty());
    assert_eq!(l.host_view(1.0).callers.len(), 1, "the host still sees the offline caller");
    assert!(!l.host_view(1.0).callers[0].online);

    connect(&mut l, "2");
    let resumed = claim(&mut l, "2", Claim::Resume { token: token.clone() });
    assert_eq!(state_of(&resumed, "2"), Some(AccessState::Granted));
    assert_eq!(l.callers().len(), 1);
    connect(&mut l, "3");
    assert_eq!(state_of(&claim(&mut l, "3", Claim::Resume { token: "nonsense".into() }), "3"), Some(AccessState::Denied));

    // Releasing ends the token for good.
    let released = l.handle("2", ClientMessage::ReleaseCaller, 1.0);
    assert_eq!(state_of(&released, "2"), Some(AccessState::Released));
    assert_eq!(state_of(&claim(&mut l, "3", Claim::Resume { token }), "3"), Some(AccessState::Denied));
}

#[test]
fn changing_the_code_drops_code_callers_only() {
    let mut l = logic(policy(false, true, true, false));
    connect(&mut l, "1");
    connect(&mut l, "2");
    let code_token = grant(&mut l, "1", code_claim(CODE));
    claim(&mut l, "2", Claim::Request { name: "Kim".into() });
    l.answer_request("2", AccessKind::Call, true);

    assert_eq!(l.set_code(AccessKind::Call, Some("short".into()), None), Err(CodeError::TooShort));
    let effects = l.set_code(AccessKind::Call, Some("N3w-C0de!".into()), None).unwrap();
    assert_eq!(state_of(&effects, "1"), Some(AccessState::Revoked));
    assert_eq!(state_of(&effects, "2"), None, "approved callers are untouched");
    assert!(l.handle("1", ClientMessage::NextCue { from_cue: None }, 1.0).iter().any(|e| matches!(e, Effect::Send(_, ServerMessage::Error { .. }))));
    connect(&mut l, "3");
    assert_eq!(state_of(&claim(&mut l, "3", Claim::Resume { token: code_token }), "3"), Some(AccessState::Denied));
    assert_eq!(state_of(&claim(&mut l, "3", code_claim(CODE)), "3"), Some(AccessState::Denied), "the old code is dead");
    grant(&mut l, "3", code_claim("N3w-C0de!"));

    // Generating one gives hex of the asked length.
    l.set_code(AccessKind::Call, None, Some(12)).unwrap();
    assert_eq!(l.code(AccessKind::Call).unwrap().len(), 12);
}

#[test]
fn revoking_and_tightening_the_policy_take_access_away() {
    let mut l = logic(policy(false, true, true, false));
    connect(&mut l, "1");
    let token = grant(&mut l, "1", code_claim(CODE));
    let grant_id = l.host_view(1.0).callers[0].grant_id.clone();
    let effects = l.revoke(&grant_id);
    assert_eq!(state_of(&effects, "1"), Some(AccessState::Revoked));
    connect(&mut l, "2");
    assert_eq!(state_of(&claim(&mut l, "2", Claim::Resume { token }), "2"), Some(AccessState::Denied));

    // Turning a method off ends the grants made with it, and pending requests.
    grant(&mut l, "2", code_claim(CODE));
    connect(&mut l, "3");
    claim(&mut l, "3", Claim::Request { name: "Kim".into() });
    let effects = l.set_policy(AccessKind::Call, policy(false, false, false, false));
    assert_eq!(state_of(&effects, "2"), Some(AccessState::Revoked));
    assert_eq!(state_of(&effects, "3"), Some(AccessState::Denied));
    assert!(effects.iter().any(|e| matches!(e, Effect::Send(to, ServerMessage::Policy { policy, .. }) if to == "3" && !policy.code)));
    assert!(l.host_view(1.0).callers.is_empty());

    // Switching to open keeps callers made any other way.
    let mut l = logic(policy(false, true, false, false));
    connect(&mut l, "1");
    grant(&mut l, "1", code_claim(CODE));
    let effects = l.set_policy(AccessKind::Call, policy(true, false, false, false));
    assert_eq!(state_of(&effects, "1"), Some(AccessState::Revoked), "the code method is off now");
}

#[test]
fn the_host_window_always_gets_in_and_nobody_else_can_pretend() {
    let mut l = logic(policy(false, false, false, false));
    connect(&mut l, "1");
    assert_eq!(state_of(&claim(&mut l, "1", Claim::Host { token: "guess".into() }), "1"), Some(AccessState::Denied));
    grant(&mut l, "1", Claim::Host { token: HOST.into() });
    l.set_policy(AccessKind::Call, policy(false, true, false, false));
    assert_eq!(l.callers().len(), 1, "policy changes never remove the host");
}

struct FakeVerifier;
impl PassVerifier for FakeVerifier {
    fn verify(&self, pass: &str, proof: &str, context: &PassContext<'_>) -> Result<VerifiedUser, String> {
        if pass == "good" && proof == format!("signed:{}", context.nonce) {
            Ok(VerifiedUser { user_id: "alex".into(), name: "Alex".into() })
        } else {
            Err("Pass not accepted".into())
        }
    }
}

#[test]
fn account_claims_go_through_the_verifier_bound_to_this_connection() {
    let mut l = logic(policy(false, false, false, true));
    let welcome = connect(&mut l, "1");
    let Effect::Send(_, ServerMessage::Welcome { nonce, .. }) = &welcome[0] else { panic!() };
    let account = |proof: String| Claim::Account { pass: "good".into(), proof };
    assert!(status_for(&claim(&mut l, "1", account(format!("signed:{nonce}"))), "1").unwrap().1.unwrap().contains("cannot check"));
    l.set_verifier(Arc::new(FakeVerifier));
    assert_eq!(state_of(&claim(&mut l, "1", account("signed:old-nonce".into())), "1"), Some(AccessState::Denied));
    let token = grant(&mut l, "1", account(format!("signed:{nonce}")));
    assert!(!token.is_empty());
    assert_eq!(l.callers()[0].name, "Alex");
}

#[test]
fn an_account_that_may_no_longer_call_loses_its_grant_and_nobody_else_does() {
    let mut l = logic(policy(false, true, false, true));
    l.set_verifier(Arc::new(FakeVerifier));
    let welcome = connect(&mut l, "1");
    let Effect::Send(_, ServerMessage::Welcome { nonce, .. }) = &welcome[0] else { panic!() };
    let token = grant(&mut l, "1", Claim::Account { pass: "good".into(), proof: format!("signed:{nonce}") });
    connect(&mut l, "2");
    let code = l.code(AccessKind::Call).unwrap().to_owned();
    grant(&mut l, "2", Claim::Code { name: "Kim".into(), code });

    assert!(l.retain_accounts(|user, _| user == "alex").is_empty(), "still allowed: nothing happens");
    let effects = l.retain_accounts(|_, _| false);
    assert_eq!(state_of(&effects, "1"), Some(AccessState::Revoked));
    assert_eq!(state_of(&effects, "2"), None, "code callers have no account");
    assert_eq!(l.callers().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Kim"]);
    assert_eq!(state_of(&claim(&mut l, "1", Claim::Resume { token }), "1"), Some(AccessState::Denied));
}

#[test]
fn joined_crew_is_remembered_after_leaving_so_a_reload_can_restore_it() {
    let mut l = logic(policy(true, false, false, false));
    connect(&mut l, "1");
    let effects = l.handle("1", ClientMessage::JoinSession { user_name: " Alex ".into(), role_id: Some("lights".into()) }, 1.0);
    assert!(matches!(&effects[0], Effect::Broadcast(ServerMessage::JoinSession { client_name, client_role, .. }) if client_name == "Alex" && client_role == "lights"));
    grant(&mut l, "1", open_claim("Alex"));
    assert!(l.snapshot().joined_users[0].is_caller);
    l.disconnect("1");
    let snapshot = l.snapshot();
    assert_eq!(snapshot.joined_users.len(), 1);
    assert!(!snapshot.joined_users[0].is_caller);
    assert!(l.host_view(1.0).people.is_empty(), "the panel lists who is connected");
}

#[test]
fn names_are_trimmed_limited_and_never_empty() {
    assert_eq!(clean_name("   "), "Guest");
    assert_eq!(clean_name("a\u{0}b\n"), "ab");
    assert_eq!(clean_name(&"x".repeat(100)).len(), MAX_NAME_CHARS);
}

#[test]
fn a_bad_initial_code_is_refused_and_a_missing_one_generated() {
    for kind in [AccessKind::View, AccessKind::Call] {
        let mut bad = settings(open_door(), open_door());
        match kind {
            AccessKind::View => bad.view_code = Some("abc".into()),
            AccessKind::Call => bad.caller_code = Some("abc".into()),
        }
        assert!(SessionLogic::new("s".into(), "sh".into(), "h".into(), bad).is_err(), "{kind:?}");
    }
    let by_code = policy(false, true, false, false);
    let l = start(AccessSettings { view: by_code, view_code: None, caller: by_code, caller_code: None });
    for kind in [AccessKind::View, AccessKind::Call] {
        assert_eq!(l.code(kind).unwrap().len(), crate::code::DEFAULT_GENERATED_CHARS);
    }
    assert_ne!(l.code(AccessKind::View), l.code(AccessKind::Call), "each gate gets a code of its own");
}

// ---------------------------------------------------------------------------------------------
// Joining
// ---------------------------------------------------------------------------------------------

fn join_claim(logic: &mut SessionLogic, id: &str, claim: Claim) -> Vec<Effect> {
    logic.handle(id, ClientMessage::RequestView { claim }, 100.0)
}

fn view_state(effects: &[Effect], id: &str) -> Option<AccessState> {
    effects.iter().find_map(|effect| match effect {
        Effect::Send(to, ServerMessage::ViewStatus { state, .. }) if to == id => Some(*state),
        _ => None,
    })
}

fn view_token(effects: &[Effect], id: &str) -> Option<String> {
    effects.iter().find_map(|effect| match effect {
        Effect::Send(to, ServerMessage::ViewStatus { resume_token, .. }) if to == id => resume_token.clone(),
        _ => None,
    })
}

/// True when these effects hand `id` anything about the show.
fn shows_anything(effects: &[Effect], id: &str) -> bool {
    effects.iter().any(|effect| match effect {
        Effect::SendShowData(to) => to == id,
        Effect::Send(to, message) => to == id && matches!(message,
            ServerMessage::GetShowData { .. } | ServerMessage::GetSessionData { .. } | ServerMessage::Callers { .. }
            | ServerMessage::CueChange { .. } | ServerMessage::JoinSession { .. }),
        _ => false,
    })
}

/// Joining protected by `view`, calling open: the hardest case for the door.
fn guarded(view: AccessPolicy) -> SessionLogic {
    start(settings(view, open_door()))
}

#[test]
fn with_joining_open_a_device_is_in_at_once_and_gets_the_show() {
    let mut l = logic(policy(false, false, true, false));
    let effects = connect(&mut l, "1");
    assert!(l.is_admitted("1"));
    assert!(effects.contains(&Effect::SendShowData("1".into())));
    assert!(shows_anything(&effects, "1"));
}

#[test]
fn a_protected_session_gives_nothing_away_and_cannot_be_entered_sideways() {
    for view in [policy(false, true, false, false), policy(false, false, true, false), policy(false, false, false, true), policy(false, false, false, false)] {
        let mut l = guarded(view);
        let welcome = connect(&mut l, "1");
        assert!(!l.is_admitted("1") && !shows_anything(&welcome, "1"), "{welcome:?}");
        let attempts = [
            ClientMessage::GetShowData,
            ClientMessage::GetSessionData,
            ClientMessage::JoinSession { user_name: "Eve".into(), role_id: None },
            // Open caller access must not become a way around the door.
            ClientMessage::ClaimCaller { claim: open_claim("Eve") },
            ClientMessage::ClaimCaller { claim: Claim::Request { name: "Eve".into() } },
            ClientMessage::ReleaseCaller,
        ];
        for message in attempts {
            let effects = l.handle("1", message, 1.0);
            assert!(!l.is_admitted("1") && !shows_anything(&effects, "1"), "{view:?} {effects:?}");
            assert!(effects.iter().all(|e| !matches!(e, Effect::Broadcast(_))), "and nothing about them reaches the others: {effects:?}");
        }
        for message in [ClientMessage::NextCue { from_cue: None }, ClientMessage::JumpToCue { cue_number: 2 }] {
            l.handle("1", message, 1.0);
        }
        assert_eq!(l.snapshot().current_cue, None);
        assert!(l.snapshot().joined_users.is_empty());
    }
}

#[test]
fn the_join_code_lets_a_device_in_and_is_not_the_caller_code() {
    let mut l = start(settings(policy(false, true, false, false), policy(false, true, false, false)));
    connect(&mut l, "1");
    let wrong = join_claim(&mut l, "1", code_claim(CODE));
    assert_eq!(view_state(&wrong, "1"), Some(AccessState::Denied), "the caller code does not open the door");
    let effects = join_claim(&mut l, "1", code_claim(VIEW_CODE));
    assert_eq!(view_state(&effects, "1"), Some(AccessState::Granted));
    assert!(view_token(&effects, "1").is_some() && shows_anything(&effects, "1"));
    assert_eq!(effects.iter().filter(|e| matches!(e, Effect::Send(_, ServerMessage::ViewStatus { .. }))).count(), 1, "told once");
    // Being in is not calling, and the join code does not make a caller.
    assert!(l.handle("1", ClientMessage::NextCue { from_cue: None }, 1.0).iter().any(|e| matches!(e, Effect::Send(_, ServerMessage::Error { .. }))));
    assert_eq!(state_of(&claim(&mut l, "1", code_claim(VIEW_CODE)), "1"), Some(AccessState::Denied));
    grant(&mut l, "1", code_claim(CODE));
}

#[test]
fn guessing_at_the_door_does_not_lock_callers_out() {
    let mut l = start(settings(policy(false, true, false, false), policy(false, true, false, false)));
    for i in 0..20 {
        let id = format!("g{i}");
        l.connect(&id, &format!("192.168.7.{i}"));
        l.handle(&id, ClientMessage::RequestView { claim: code_claim("000000") }, 1.0);
    }
    let view = l.host_view(1.0);
    assert!(view.view_paused_seconds.is_some() && view.call_paused_seconds.is_none());
}

#[test]
fn a_join_request_waits_for_the_host_who_can_also_put_the_device_out_again() {
    let mut l = guarded(policy(false, false, true, false));
    connect(&mut l, "1");
    assert_eq!(view_state(&join_claim(&mut l, "1", Claim::Request { name: "Robin".into() }), "1"), Some(AccessState::Pending));
    assert_eq!(l.host_view(1.0).requests, vec![HostRequest { client_id: "1".into(), name: "Robin".into(), kind: AccessKind::View }]);
    // Approving the wrong thing does nothing.
    assert!(l.answer_request("1", AccessKind::Call, true).is_empty());

    let approved = l.answer_request("1", AccessKind::View, true);
    assert_eq!(view_state(&approved, "1"), Some(AccessState::Granted));
    assert!(shows_anything(&approved, "1"));
    let token = view_token(&approved, "1").unwrap();
    let person = &l.host_view(1.0).people[0];
    assert!(person.admitted && person.name.as_deref() == Some("Robin"));
    grant(&mut l, "1", open_claim("Robin"));

    let removed = l.remove_device("1");
    assert_eq!(view_state(&removed, "1"), Some(AccessState::Revoked));
    assert_eq!(state_of(&removed, "1"), Some(AccessState::Revoked), "the caller role goes with it");
    assert!(!l.is_admitted("1") && l.callers().is_empty());
    connect(&mut l, "2");
    assert_eq!(view_state(&join_claim(&mut l, "2", Claim::Resume { token }), "2"), Some(AccessState::Denied));

    connect(&mut l, "3");
    join_claim(&mut l, "3", Claim::Request { name: "Eve".into() });
    assert_eq!(view_state(&l.answer_request("3", AccessKind::View, false), "3"), Some(AccessState::Denied));
    assert!(!l.is_admitted("3"));
}

#[test]
fn a_join_token_survives_a_reload_and_a_caller_token_or_the_host_gets_in_too() {
    let mut l = start(settings(policy(false, false, true, false), policy(false, false, true, false)));
    connect(&mut l, "1");
    join_claim(&mut l, "1", Claim::Request { name: "Kim".into() });
    let view = view_token(&l.answer_request("1", AccessKind::View, true), "1").unwrap();
    claim(&mut l, "1", Claim::Request { name: "Kim".into() });
    let call = status_for(&l.answer_request("1", AccessKind::Call, true), "1").unwrap().2.unwrap();
    l.disconnect("1");

    // The crew page comes back with its join token.
    connect(&mut l, "2");
    let effects = join_claim(&mut l, "2", Claim::Resume { token: view });
    assert!(view_state(&effects, "2") == Some(AccessState::Granted) && shows_anything(&effects, "2"));
    // The caller view only has the caller token: that must be enough.
    connect(&mut l, "3");
    let effects = claim(&mut l, "3", Claim::Resume { token: call });
    assert!(state_of(&effects, "3") == Some(AccessState::Granted) && shows_anything(&effects, "3"), "{effects:?}");
    // And the host's own window, which never joins.
    connect(&mut l, "4");
    let effects = claim(&mut l, "4", Claim::Host { token: HOST.into() });
    assert!(l.is_admitted("4") && shows_anything(&effects, "4"));
    connect(&mut l, "5");
    assert!(!shows_anything(&claim(&mut l, "5", Claim::Host { token: "guess".into() }), "5"));
    assert_eq!(view_state(&join_claim(&mut l, "5", Claim::Host { token: HOST.into() }), "5"), Some(AccessState::Denied));

    // A caller who only had their caller token leaves the session when they stop calling.
    let released = l.handle("3", ClientMessage::ReleaseCaller, 1.0);
    assert_eq!(view_state(&released, "3"), Some(AccessState::Revoked));
}

#[test]
fn protecting_a_running_session_puts_the_uninvited_out_and_opening_it_lets_them_in() {
    let mut l = logic(policy(false, false, true, false));
    connect(&mut l, "crew");
    connect(&mut l, "caller");
    grant(&mut l, "caller", Claim::Host { token: HOST.into() });

    let effects = l.set_policy(AccessKind::View, policy(false, false, true, false));
    assert_eq!(view_state(&effects, "crew"), Some(AccessState::Revoked));
    assert_eq!(view_state(&effects, "caller"), None, "callers stay");
    assert!(!l.is_admitted("crew") && l.is_admitted("caller"));
    assert!(effects.iter().any(|e| matches!(e, Effect::Send(to, ServerMessage::Policy { view_policy, .. }) if to == "crew" && view_policy.request)));

    join_claim(&mut l, "crew", Claim::Request { name: "Sam".into() });
    let effects = l.set_policy(AccessKind::View, open_door());
    assert_eq!(view_state(&effects, "crew"), Some(AccessState::Granted), "{effects:?}");
    assert!(shows_anything(&effects, "crew") && l.host_view(1.0).requests.is_empty());
}

#[test]
fn new_settings_apply_in_one_step_and_only_a_changed_code_removes_people() {
    let by_code = policy(false, true, false, false);
    let mut l = start(settings(by_code, by_code));
    connect(&mut l, "1");
    join_claim(&mut l, "1", code_claim(VIEW_CODE));
    grant(&mut l, "1", code_claim(CODE));

    // Saving what is already in force removes nobody.
    let same = l.settings();
    let effects = l.apply_settings(same.clone()).unwrap();
    assert!(view_state(&effects, "1").is_none() && state_of(&effects, "1").is_none(), "{effects:?}");
    assert!(l.apply_settings(AccessSettings { view_code: Some("short".into()), ..same.clone() }).is_err());
    assert_eq!(l.settings(), same, "a refused change changes nothing");

    let effects = l.apply_settings(AccessSettings { view_code: Some("OtherDoor1".into()), ..same }).unwrap();
    assert_eq!(state_of(&effects, "1"), None, "still a caller");
    assert!(l.is_admitted("1"), "and so still in");
    connect(&mut l, "2");
    assert_eq!(view_state(&join_claim(&mut l, "2", code_claim(VIEW_CODE)), "2"), Some(AccessState::Denied));
    assert_eq!(view_state(&join_claim(&mut l, "2", code_claim("OtherDoor1")), "2"), Some(AccessState::Granted));
}

/// Members may join; only "alex" is also a caller.
struct ByKind;
impl PassVerifier for ByKind {
    fn verify(&self, pass: &str, _proof: &str, context: &PassContext<'_>) -> Result<VerifiedUser, String> {
        if context.kind == AccessKind::Call && pass != "alex" {
            return Err("Not a caller".into());
        }
        Ok(VerifiedUser { user_id: pass.into(), name: pass.into() })
    }
}

#[test]
fn account_access_tells_joining_apart_from_calling_and_ends_with_membership() {
    let by_account = policy(false, false, false, true);
    let mut l = start(settings(by_account, by_account));
    l.set_verifier(Arc::new(ByKind));
    let account = |who: &str| Claim::Account { pass: who.into(), proof: "p".into() };
    for (id, who) in [("1", "alex"), ("2", "kim")] {
        connect(&mut l, id);
        assert_eq!(view_state(&join_claim(&mut l, id, account(who)), id), Some(AccessState::Granted));
    }
    grant(&mut l, "1", account("alex"));
    assert_eq!(state_of(&claim(&mut l, "2", account("kim")), "2"), Some(AccessState::Denied));

    // Kim leaves the show; Alex stays a member but stops being a caller.
    let effects = l.retain_accounts(|user, kind| user == "alex" && kind == AccessKind::View);
    assert_eq!(view_state(&effects, "2"), Some(AccessState::Revoked));
    assert_eq!(state_of(&effects, "1"), Some(AccessState::Revoked));
    assert!(l.is_admitted("1") && !l.is_admitted("2"));
}
