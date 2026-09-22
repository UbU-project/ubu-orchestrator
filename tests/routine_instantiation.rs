use serde_json::json;
use ubu_core::{ObjectType, UbuId, UbuTimestamp};
use ubu_orchestrator::services::routine_instantiation::{
    instantiate, Instantiation, RoutineDefinition,
};
fn sec(t: &str) -> u64 {
    UbuTimestamp::parse(t).unwrap().inner().unix_timestamp() as u64
}
fn def(title: &str, start: &str, end: Option<&str>, duration: u64) -> RoutineDefinition {
    RoutineDefinition { objective_id:UbuId::new(ObjectType::Objective), schedule:serde_json::from_value(json!({"timezone":"America/New_York","rule":{"kind":"daily"},"schedule_version":1})).unwrap(), template:serde_json::from_value(json!({"title":title,"nominal_start":start,"placement":if end.is_some(){"planned"}else{"static"},"allowed_local_range":end.map(|end|json!({"earliest":start,"latest":end})),"duration_estimate":{"type":"fixed","seconds":duration},"template_version":1})).unwrap() }
}
fn after(b: &mut RoutineDefinition, a: &RoutineDefinition, offset: i64) {
    b.template.after.push(
        serde_json::from_value(json!({"objective_id":a.objective_id,"offset_seconds":offset}))
            .unwrap(),
    );
}
fn run(defs: &[RoutineDefinition], a: &str, b: &str) -> Instantiation {
    instantiate(defs, sec(a), sec(b))
}
#[test]
fn check_in_chain_is_predecessor_first_and_skips_wednesday() {
    let mut a = def("Check-in 1", "09:00:00", Some("11:45:00"), 300);
    let mut b = def("Check-in 2", "12:00:00", Some("14:00:00"), 300);
    let mut c = def("Check-in 3", "14:00:00", Some("17:00:00"), 300);
    after(&mut b, &a, 3600);
    after(&mut c, &b, 3600);
    for d in [&mut a, &mut b, &mut c] {
        d.schedule.rule =
            serde_json::from_value(json!({"kind":"weekly","weekdays":["mon","tue","thu","fri"]}))
                .unwrap();
    }
    let out = run(
        &[c.clone(), b.clone(), a.clone()],
        "2026-09-22T12:00:00Z",
        "2026-09-23T22:00:00Z",
    );
    let os: Vec<_> = out
        .occurrences
        .iter()
        .filter(|o| o.local_date == "2026-09-22")
        .collect();
    assert_eq!(os.len(), 3);
    for (o, s, e) in [
        (os[0], "13:00", "15:45"),
        (os[1], "16:00", "18:00"),
        (os[2], "18:00", "21:00"),
    ] {
        assert_eq!(
            (o.start, o.end),
            (
                sec(&format!("2026-09-22T{s}:00Z")),
                sec(&format!("2026-09-22T{e}:00Z"))
            )
        );
    }
    assert!(os[0].key.ends_with("/s1/2026-09-22T09:00:00/planned/t1"));
    assert_eq!(os[1].after, vec![(a.objective_id, 3600)]);
    assert_eq!(os[2].after, vec![(b.objective_id, 3600)]);
    assert!(!out.occurrences.iter().any(|o| o.local_date == "2026-09-23"));
}
#[test]
fn lowering_binds_and_infeasibility_leaves_unmatched_successor() {
    let a = def("A", "08:00:00", Some("12:00:00"), 1800);
    let mut b = def("B", "08:00:00", Some("10:00:00"), 1800);
    let mut s = def("S", "08:00:00", None, 600);
    after(&mut b, &a, 3600);
    after(&mut s, &b, 0);
    let inspect =
        |defs: &[RoutineDefinition]| run(defs, "2026-09-22T12:00:00Z", "2026-09-22T22:00:00Z");
    let out = inspect(&[s.clone(), b.clone(), a.clone()]);
    let os: Vec<_> = out
        .occurrences
        .iter()
        .filter(|o| o.local_date == "2026-09-22")
        .collect();
    assert_eq!(os[1].start, sec("2026-09-22T13:30:00Z"));
    assert_eq!(
        (os[2].start, os[2].end),
        (sec("2026-09-22T14:00:00Z"), sec("2026-09-22T14:10:00Z"))
    );
    b.template.after[0].offset_seconds = 10800;
    let out = inspect(&[s.clone(), b.clone(), a]);
    assert!(!out
        .occurrences
        .iter()
        .any(|o| o.objective_id == b.objective_id));
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "routine_after_infeasible"));
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "routine_after_unmatched"));
    assert_eq!(
        out.occurrences
            .iter()
            .find(|o| o.objective_id == s.objective_id && o.local_date == "2026-09-22")
            .unwrap()
            .start,
        sec("2026-09-22T12:00:00Z")
    );
}
#[test]
fn dst_gap_and_overlap_have_recorded_policy() {
    let a = def("Gap", "02:30:00", None, 300);
    let out = run(&[a], "2026-03-08T05:00:00Z", "2026-03-09T03:59:00Z");
    assert!(!out.occurrences.iter().any(|o| o.local_date == "2026-03-08"));
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "routine_occurrence_nonexistent_local_time"));
    let a = def("Overlap", "01:30:00", None, 300);
    let out = run(&[a], "2026-11-01T04:00:00Z", "2026-11-03T04:59:00Z");
    assert_eq!(
        out.occurrences
            .iter()
            .find(|o| o.local_date == "2026-11-01")
            .unwrap()
            .start,
        sec("2026-11-01T05:30:00Z")
    );
    assert_eq!(
        out.occurrences
            .iter()
            .find(|o| o.local_date == "2026-11-02")
            .unwrap()
            .start,
        sec("2026-11-02T06:30:00Z")
    );
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "routine_occurrence_ambiguous_local_time"));
}
#[test]
fn calendar_rules_bounds_and_exdates() {
    for (rule, start, end, expected) in [
        (
            json!({"kind":"first_workday_of_month"}),
            "2026-07-30T00:00:00Z",
            "2026-10-15T00:00:00Z",
            vec!["2026-08-03", "2026-09-01", "2026-10-01"],
        ),
        (
            json!({"kind":"first_workday_of_quarter"}),
            "2026-06-01T00:00:00Z",
            "2027-01-15T00:00:00Z",
            vec!["2026-07-01", "2026-10-01", "2027-01-01"],
        ),
        (
            json!({"kind":"monthly_day","days":[31]}),
            "2026-09-01T00:00:00Z",
            "2026-11-01T00:00:00Z",
            vec!["2026-10-31"],
        ),
        (
            json!({"kind":"daily"}),
            "2026-09-01T00:00:00Z",
            "2026-10-01T00:00:00Z",
            vec!["2026-09-23", "2026-09-25", "2026-09-26"],
        ),
    ] {
        let mut a = def("Rule", "09:00:00", None, 300);
        a.schedule.rule = serde_json::from_value(rule).unwrap();
        if expected.len() == 3 && expected[0] == "2026-09-23" {
            a.schedule.enabled_from = Some("2026-09-23".into());
            a.schedule.enabled_until = Some("2026-09-26".into());
            a.schedule.exdates = vec!["2026-09-24".into()];
        }
        let out = run(&[a], start, end);
        assert_eq!(
            out.occurrences
                .iter()
                .filter(|o| o.start < sec(end) && o.end > sec(start))
                .map(|o| o.local_date.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
}
#[test]
fn cycles_include_downstream_and_unknown_zones() {
    let mut a = def("A", "09:00:00", None, 300);
    let mut b = def("B", "09:00:00", None, 300);
    let mut c = def("C", "09:00:00", None, 300);
    after(&mut a, &b, 0);
    after(&mut b, &a, 0);
    after(&mut c, &b, 0);
    let mut ids = vec![
        a.objective_id.to_string(),
        b.objective_id.to_string(),
        c.objective_id.to_string(),
    ];
    ids.sort();
    let out = run(&[c, b, a], "2026-09-22T00:00:00Z", "2026-09-23T00:00:00Z");
    assert_eq!(out.diagnostics.len(), 1);
    assert_eq!(out.diagnostics[0].code, "routine_after_cycle");
    assert!(out.diagnostics[0].message.ends_with(
        &ids.iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    assert!(out.occurrences.iter().all(|o| o.after.is_empty()));
    let mut a = def("Zone", "09:00:00", None, 300);
    a.schedule.timezone = "America/Nowhere".into();
    let out = run(&[a], "2026-09-22T00:00:00Z", "2026-09-23T00:00:00Z");
    assert!(out.occurrences.is_empty());
    assert_eq!(out.diagnostics[0].code, "routine_timezone_unknown");
}
