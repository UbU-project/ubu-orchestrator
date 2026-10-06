#[path = "support/precondition_fixture.rs"]
mod fixture;
use fixture::*;
use serde_json::json;
use ubu_orchestrator::services::{precondition_advisor, suggest_tags};

fn id(n: usize) -> String {
    format!("task_018f3c8e9b2a7c4d8f1e2a3b4c5c{:04x}", n + 100)
}

#[tokio::test]
async fn precondition_skips_name_three_then_count_twenty_seven_without_identifiers() {
    for total in [3, 30] {
        let (state, _) = ready(tree()).await;
        fact(&state, 0.0).await;
        for n in 0..total {
            seed(&state, &id(n), "active", json!({"title":" \t"})).await;
        }
        let (_, diagnostics) = precondition_advisor::select(&state, 25).await.unwrap();
        assert_eq!(diagnostics.len(), if total == 3 { 3 } else { 4 });
        for (n, diagnostic) in diagnostics.iter().take(3).enumerate() {
            assert_eq!(diagnostic.code, "advisory_task_skipped");
            assert!(diagnostic.message.contains(&id(n)));
        }
        if total > 3 {
            assert_eq!(diagnostics[3].message,"27 more Tasks were skipped: they are routine occurrences or have neither a title nor a description");
            assert!(
                !diagnostics[3].message.contains("task_")
                    && !diagnostics[3].message.contains(TARGET)
            );
        }
    }
    assert_eq!(suggest_tags::MAX_LIMIT, 25);
}

#[tokio::test]
async fn tag_occurrence_skips_name_three_then_count_twenty_seven_without_identifiers() {
    for total in [3, 30] {
        let (state, _) = ready(tree()).await;
        for n in 0..total {
            seed(&state,&id(n),"active",json!({"occurrence":{"routine_objective_id":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01","local_date":"2026-09-29","key":format!("synthetic-occurrence-{n}")}})).await;
        }
        let diagnostics = suggest_tags::skipped_occurrences(&state).await.unwrap();
        assert_eq!(diagnostics.len(), if total == 3 { 3 } else { 4 });
        for (n, diagnostic) in diagnostics.iter().take(3).enumerate() {
            assert_eq!(diagnostic.code, "suggest_tags_occurrence_skipped");
            assert!(diagnostic.message.contains(&id(n)));
        }
        if total > 3 {
            assert_eq!(diagnostics[3].message,"27 more routine occurrences were skipped for the same reason; a routine's category belongs on its template, which the Routines screen edits");
            assert!(
                !diagnostics[3].message.contains("task_")
                    && !diagnostics[3].message.contains(TARGET)
            );
        }
    }
}

#[tokio::test]
async fn controller_missing_targets_yield_three_names_and_one_count_only_diagnostic() {
    let (state, stub) =
        ready(json!({"target":"facts.synthetic.missing","predicate":"equals","expected":true}))
            .await;
    fact(&state, 0.0).await;
    for n in 0..9 {
        seed(&state, &id(n), "active", json!({})).await;
    }
    let before = canonical_rows(&state).await;
    let ledger = count(&state, "mutation_envelopes").await;
    let mut result = candidate_batches(&state, stub.as_ref()).await;
    precondition_advisor::vet_result(&state, &mut result).await.unwrap();
    assert!(result.proposed_candidates.is_empty());
    assert_eq!(count(&state, "advisory_candidates").await, 0);
    assert_eq!(count(&state, "mutation_envelopes").await, ledger);
    assert_eq!(canonical_rows(&state).await, before);
    let diagnostics = &result.diagnostics;
    assert_eq!(diagnostics.len(), 4);
    for (n, diagnostic) in diagnostics.iter().take(3).enumerate() {
        let message = diagnostic["message"].as_str().unwrap();
        assert!(message.contains(&id(n)) && message.contains("facts.synthetic.missing"));
    }
    assert_eq!(
        diagnostics[3],
        json!({"code":"precondition_missing_targets","message":"7 more Tasks need recorded targets; no candidates were enqueued for those Tasks."})
    );
    let message = diagnostics[3]["message"].as_str().unwrap();
    assert!(!message.contains("task_") && !message.contains("facts.synthetic.missing"));
}
