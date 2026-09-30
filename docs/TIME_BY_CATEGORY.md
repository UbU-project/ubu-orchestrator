# Time by category

`GET /reports/time-by-category?schema_version=ubu.orchestrator.time_by_category.v1&from=…&to=…`
answers the question Quick UbU's `report` answered daily: where did the time
go, by category. Both bounds are inclusive RFC 3339 instants. Absent, `to`
is now and `from` is seven days before `to`, Quick UbU's `--days 7`. `from`
after `to` is HTTP 400 `time_by_category_invalid_range`; a bound that is not
an instant is `time_by_category_invalid_bound`.

```json
{"schema_version":"ubu.orchestrator.time_by_category.v1","generated_at":"…","from":"…","to":"…",
 "categories":[{"category":"work","seconds":9000,"static_seconds":9000,"completed_seconds":0,"task_count":2}],
 "unmeasured":[{"task_id":"task_…","title":"…","reason":"…"}],
 "total_seconds":9000}
```

Rows are sorted by `seconds` descending, then `category` ascending.
`Uncategorized` is an ordinary row: how much of the week is uncategorised is
the point of running the report.

## The two contributions

Quick UbU's `report_by_category` rule, with one change mainline needs.

- **Static.** An active or completed Task with a `static_window` contributes
  the overlap of that window with the range, when it is positive. It is not
  gated on completion: time inside a fixed window is spent whether or not
  anyone ticked a box.
- **Dynamic.** A Task whose status is `completed`, with no `static_window`,
  whose **latest** completion was recorded inside the range, contributes the
  observed window's length when the completion carries one, and otherwise
  its estimate: `seconds` for a fixed estimate, `mode_seconds` for a
  `shifted_lognormal_p95` one. The mode is the honest single number for a
  skewed estimate; the p95 would inflate every retrospective. With neither,
  the Task contributes no seconds and is named in `unmeasured` with the
  reason, because a report that quietly omits work is worse than one that
  says what it could not measure.

The latest completion is found by `calendar_interaction::latest_completion`,
the same query the Calendar uses, so the report and the Calendar cannot
disagree about which completion counts.

Routine occurrences count like any other Task: the time was spent.

## Once, from the latest completion

Mainline has `POST /task/{id}/reopen`, which Quick UbU did not. A reopened
Task keeps its completion log, so counting every completion log would count
a complete, undo, complete twice. The rule is: **a Task contributes at most
once, from its latest completion, and only while its status is
`completed`.** A reopened Task that is left active contributes nothing until
it is completed again. Two completion logs, one contribution.

The related defect, that `apply_completed_effects` runs again on a second
completion so effects are applied twice, is unchanged by this and still
needs a decision before the freeze.
