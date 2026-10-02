# Category colour

Only an explicit operator action, an import carrying an explicit category, or a
future reviewed candidate of a dedicated category kind may set `category_tag`.
The `add_tag` applier never sets it. Tags never derive or rewrite a category.

Each calendar step exposes its Task's category and looks up `gcal_color_id` using
only `category_tag`. An absent or unmapped category has no colour field; an
unmapped category is still included. There is no fallback to another tag.
Thus a Task tagged `commute` and `work`, categorized `commute`, has colour `7`.

Defaults: personal 3, relationship 5, business 6, committed 11, sleep 8,
entertainment 1, grocery 2, commute 7, undefined 4, education_house 10, work 9.

**`location` is retired** from P1B-54. Until then it was the default for
colour 8. The operator's own words: *"the `location` category is depreciated
as that is a Google Calendar Event that represents a `UniverseState` value of
geographic location, and was only a rubric for manual estimation of travel."*
Where the operator is belongs in `UniverseState`, as a fact a precondition can
read. It does not belong in a category, which says what kind of time a Task
is.

`sleep` takes colour 8, Graphite. Three things follow:

- Graphite is one category, so a real event coloured Graphite is captured as
  `sleep`, and the Settings inverse table reports colour 8 as `mapped`;
- a routine in the `sleep` category exports its events in Graphite with no
  Setting. `calendar.color.sleep` is needed only to choose another colour;
- `sleep` is in the palette, so the app's Colours card and its Routines
  Category select both offer it.

An operator who had set `calendar.color.location` keeps it. Settings override
defaults, and retiring a default does not delete an operator's record: the
category `location` stays in that store's palette with origin `setting`, at
the colour the operator gave it. If that colour is 8, it now shares Graphite
with `sleep`, and the inverse table reports the collision. Reverting the
Setting removes `location` from that palette.

The `suggest_tags` prompt lists the default category names. It no longer
offers `location` and now offers `sleep`.

Set `UBU_CATEGORY_PALETTE_PATH` to an operator-owned JSON object such as
`{"commute":"6","study":"10"}` to merge overrides over these defaults at startup.
Keys are exact and case-sensitive. Values must be strings `"1"` through `"11"`.
Invalid files cause a startup configuration error identifying the bad entry.
Several categories may share a colour; resolving that ambiguity on import belongs
to the future Google Calendar adapter.

Colour discloses category just as a summary discloses its content. Any surface
that redacts a step's summary must also omit `category_tag` and `gcal_color_id`.
The calendar response currently has no redaction path.

Quick UbU painted only fixed events. This service supplies colour metadata for
all steps; whether planner-placed steps get painted is a future Google Calendar
projection decision.

Non-capacity Static steps are present in calendar and candidate steps. Kernel
probability, robustness and legitimization cover capacity work only; risk and
plan-quality analysis sees the merged candidate steps. In repair, frozen steps
retain their complete prior representation and win on Task id.

Static conflict clause (b) is read literally: a Static prerequisite that ends too
late is a precedence conflict even if it does not consume capacity. From P1B-54
that conflict is a warning and the edge is dropped; see
[STATIC_CONTAINMENT.md](STATIC_CONTAINMENT.md). Non-capacity
steps never cause an occupancy conflict. A pair is reported once if it violates
both rules, ordered by the earlier `(start, id)` and then the other id.

Planning now uses Unix seconds and a bounded now-based default horizon; see
[PLANNING_TIME.md](PLANNING_TIME.md). The kernel still preserves prior Static
steps in repair even when their window has changed.
