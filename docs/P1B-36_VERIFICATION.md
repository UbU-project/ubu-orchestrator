# P1B-36 verification

Base: `a8d04186c9f59439f96e1c0c919c2b14a1c31b3a`, clean. Only `ubu-orchestrator` changes. Branch: `p1b-36-preference-authoring`. One signed commit per section: A `6368d1d`, B `65e8e86`, C `42c0ade`, D `f71886f`, and E is the commit containing this report and the new test file. The final full E revision is supplied in the completion response; a document cannot embed its own commit hash.

## Results and method

- **304 passed, 0 failed, 0 ignored**: 294 existing tests plus exactly ten new integration tests in `tests/preference_authoring.rs`. No existing tests were changed. The baseline and each A–D checkpoint passed all 294 existing tests before committing.
- **Clippy 10 before, 10 after; delta 0.** Both measurements used `cargo clippy --locked --offline --all-targets --message-format=json` in this checkout during this ticket run with the same sanitized environment. Count distinct `compiler-message` warnings by `(code, message, primary spans' file, line, column)`, deduplicating repeated library/library-test warnings. This is a count comparison, not a claim that line numbers stayed unchanged.
- `Cargo.lock` is byte-identical to baseline. SHA-256: `52f59663fd401805f08baa11ca5a71360b25fe2cb57e4bf749379d1ae0fa1cc1`. `Cargo.toml` is unchanged; no dependency was added and no pin moved.
- All eleven sibling repositories retain their initial HEAD and clean status, including the seven read-only dependencies/reference repositories named by the ticket. No real Quick UbU data was read or committed.
- Tests run under `env -i PATH=… HOME=… CARGO_NET_OFFLINE=true`, through an inherited seccomp filter denying IPv4/IPv6 socket creation, with `cargo test --locked --offline`. The final full suite additionally ran under `strace -f -qq -e trace=network`: **0 Internet-family socket mentions and 0 connect calls**. HTTP tests invoke the Axum router in process. All data and titles are synthetic. Git is the only network operation needed to publish this ticket; no credentials appear in fixtures or evidence.
- OpenAPI was regenerated using `cargo run --locked --offline --example generate_openapi` and contains all four endpoint operations and their request/response shapes.
- Only the three new Rust files were passed to `rustfmt --edition 2021`; each passes its individual formatting check. Existing files were not reformatted. `git diff --check` passes. No compiler or runtime resource cap was introduced.

## Request values and rejection evidence

The tests call the production `build_request_from_store` and assert each request Task's `value`, then exercise Plan generation. They do not merely assert the explanation values in a response. The JSON below is copied verbatim from the final full test run; synthetic generated IDs vary between runs.

### Test 1: captured pair

```json
{"request_values":{"task_01a0e0db28767763991b9a072b80bcf3":1.0,"task_01a0e0db288178b290c6e06048a26c70":0.1},"task_a":"task_01a0e0db28767763991b9a072b80bcf3","task_b":"task_01a0e0db288178b290c6e06048a26c70"}
```

The test also checks `user_defined`, the full RFC 3339 acquisition timestamp, user provenance without `source`, readable listing titles, and a fresh title after a Task rename.

### Test 2: three-Task chain

```json
{"chain":["task_01a0e0db2869709191696ea332025444","task_01a0e0db28787c2390c4f29d80b0af31","task_01a0e0db287e78539d399a484202b1ec"],"request_values":{"task_01a0e0db2869709191696ea332025444":1.0,"task_01a0e0db28787c2390c4f29d80b0af31":0.55,"task_01a0e0db287e78539d399a484202b1ec":0.1}}
```

### Test 6: cycle rejected, exactly two statements remain

```json
{"preference_count":2,"rejection":{"diagnostics":[{"code":"preference_cycle_rejected","message":"Preference cycle among Tasks [task_01a0e0db286e745099b93538dfa5be16 -> task_01a0e0db28807d218a1445be272f7c2d -> task_01a0e0db287b7eb18a9b3fcebd82e2b1 -> task_01a0e0db286e745099b93538dfa5be16]; disable or delete a conflicting Preference first"}],"error":"Preference cycle among Tasks [task_01a0e0db286e745099b93538dfa5be16 -> task_01a0e0db28807d218a1445be272f7c2d -> task_01a0e0db287b7eb18a9b3fcebd82e2b1 -> task_01a0e0db286e745099b93538dfa5be16]; disable or delete a conflicting Preference first"}}
```

The third edge is rejected with HTTP 400, and both the Preference count and mutation-envelope count remain unchanged. The test deliberately chooses a non-ID-order chain and verifies every consecutive diagnostic member is an actual directed edge, with the first Task repeated to close the witness.

### Test 7: disable without deleting

```json
{"before":{"task_01a0e0db28767763991b99c82e598ccc":1.0,"task_01a0e0db287f75d3a5126c75a424d218":0.1,"task_01a0e0db288378009f4762d024ac37d8":0.1},"disabled":{"task_01a0e0db28767763991b99c82e598ccc":0.1,"task_01a0e0db287f75d3a5126c75a424d218":0.1,"task_01a0e0db288378009f4762d024ac37d8":0.1},"listing":{"preferences":[{"acquired_date":"2026-09-28T09:00:00Z","enabled":false,"order":"a_preferred_to_b","preference_id":"pref_01a0e0db288a7c6381bd62fd08c5f881","task_a":"task_01a0e0db28767763991b99c82e598ccc","task_a_title":"Synthetic A","task_b":"task_01a0e0db287f75d3a5126c75a424d218","task_b_title":"Synthetic B","version":2}],"schema_version":"ubu.orchestrator.preference.v1"},"task_a":"task_01a0e0db28767763991b99c82e598ccc","task_b":"task_01a0e0db287f75d3a5126c75a424d218"}
```

The disabled statement remains in the list with both IDs, both current titles, timestamp and incremented version. A stale PATCH returns 409 with `version_conflict`; missing `expected_version` is rejected. Re-enabling with the current version restores the ranking when the graph is consistent.

### Test 9: real withdrawal

```json
{"before":{"task_01a0e0db2869709191696e87e9dc20a2":1.0,"task_01a0e0db28727623b79b730a42698ee2":0.1,"task_01a0e0db28767763991b99d6fdaffa2e":0.1},"deleted":{"task_01a0e0db2869709191696e87e9dc20a2":0.1,"task_01a0e0db28727623b79b730a42698ee2":0.1,"task_01a0e0db28767763991b99d6fdaffa2e":0.1},"task_a":"task_01a0e0db2869709191696e87e9dc20a2","task_b":"task_01a0e0db28727623b79b730a42698ee2"}
```

DELETE returns 204 with an empty body, the Preference row is absent, listing is empty, and the next request loses the ranking. A Task ID sent to this deletion route returns 404 and leaves the Task untouched.

### Test 10: completed subject

```json
[{"code":"preference_ignored_unknown_task","message":"Preference `pref_01a0e0db28827f52a1f9e662ad371a52` is ignored because Task `task_01a0e0db286b7f8286d3cef1f2e1a4f8` is absent from the eligible planning set"}]
```

The Preference payload and version remain unchanged after completion and generation. Its remaining eligible subject becomes unranked. Disabling the Preference suppresses the ignored-subject diagnostic on the next Plan.

The other tests cover all six rejection codes, unknown and inactive Task IDs, forbidden client attribution fields, required/known schema versions, reversed indifference duplicates, strict-versus-indifferent contradictions, and a cycle that becomes invalid only when a disabled edge is re-enabled. Rejected writes leave canonical admissions unchanged.

## Reuse and judgment calls

`task_priority::layer_preferences` is reused **as-is for detection**: its enabled filtering, indifference merging, strongly connected components, cycle memberships, layering and value calculation are unchanged. Its public `cycles` vector lists component members in ID order rather than relationship order. No detection algorithm was extracted or duplicated. The small `cycle_witness` helper in the same module walks edges inside an already detected component, starting with a strict edge and reconstructing a return path. It supplies a deterministic, closed, relationship-ordered explanation, including mixed strict/indifferent graphs. A component containing several branching cycles is explained by one real cycle, rather than claiming all members lie on one simple cycle.

No disagreement with the ten judgment calls. The implementation rejects cycles, contradictions and duplicates; supports disable separately from physical deletion; rejects Objective authoring; fixes native attribution to the user; validates active Task subjects; validates only enabled existing statements; and lists without pagination. The existing `preference_cycle` code and message are unchanged. A repository search including tests was recorded before the planning addition; no existing diagnostic was renamed or removed.

Literal readings and boundaries:

- Pair identity is semantic: reversed indifference is the same pair/order. A strict relation versus indifference on that pair is a contradiction, just as the opposite strict direction is. Duplicate checks precede contradiction checks. Indifference otherwise retains exactly the existing layering behaviour.
- New statements are always enabled. A disabled existing statement does not block a new one; re-enabling validates subjects and the entire enabled graph again. This graph includes Task subjects of active, enabled stored Preferences even when those Tasks are not currently eligible for planning. An imported enabled cycle can block a new enabled statement until it is disabled or deleted. Imported Objective relations are ignored by the existing graph logic.
- Creation and enable/disable use ordinary user-authority store admission. The service takes the existing import lock followed by the Task-action lock so concurrent endpoint writes and Quick UbU imports do not validate against each other's half-finished changes. Existing store preconditions still enforce the optimistic Preference version.
- The store has no public deletion helper, and this ticket prohibits changing another repository. DELETE therefore performs a narrowly typed `DELETE FROM objects … object_type='Preference'` under those same locks. It withdraws the row with no tombstone or retained Preference. The Device mutation ledger is retained as infrastructure; deletion adds no fabricated work Log. It is not a general object deletion route.
- POST and PATCH require the Preference schema version and reject fields outside their allow-lists. GET and bodyless DELETE do not accept a JSON schema-version field. PATCH requires `expected_version`; unknown/non-Preference IDs return 404. Repeating DELETE after withdrawal also returns 404.
- Listing includes `preference_id` and `version` so an operator can submit a versioned PATCH. Missing Task titles are null. Already imported Objective pairs remain visible with Objective IDs and null Task fields; this does not enable their authoring or planning effect.
- Plan-time diagnostics use the exact eligible set passed to layering, so horizon, lifecycle, mandatory-occurrence and other eligibility exclusions all qualify. Each absent subject of an enabled Task Preference is named; two absent subjects produce two diagnostics. The canonical Preference is never rewritten. Explicit caller-supplied planning requests retain their existing bypass of store-derived layering.

## Known limits

1. **Pairwise only.** There is no way to say "these five are all more important than those three" except as pairs, and the number of pairs grows quickly.
2. **No strength.** `a_preferred_to_b` says which, never by how much. The value gradient comes from the layering, not from the operator.
3. **`a_indifferent_to_b` is accepted and stored but carries no special handling here** beyond what `layer_preferences` already does with it.
4. **Objective Preferences are unreachable.** The schema allows them; planning does not read them; this endpoint rejects them.
5. **Cycle rejection is write-time only for this endpoint.** A cycle arriving through `/import/quick-ubu` is still only reported at plan time.
6. **No bulk authoring.** Ranking a fresh backlog means one call per pair.
7. **Titles in the listing are a snapshot.** Renaming a Task changes what the next listing shows; nothing is cached, but nothing is versioned either.

