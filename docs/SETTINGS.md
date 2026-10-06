# Settings and the effective category palette

[UBU-D0287](../../ubu-design/DECISIONS.md#ubu-d0287-named-operator-settings-are-setting-records-not-preferences)
identifies named configuration values as `Setting` records with `id`, `name`,
`value` and `authority_source`. They are **not Preferences**: they express no
pairwise value judgment and never enter Preference layering.

Native colour authoring uses names `calendar.color.<category>`. Namespaces keep a
category such as `work` distinct from future configuration with another meaning.
Category names are case-sensitive. Any nonblank category suffix is allowed; the
current authoring routes reject other namespaces with `setting_unknown_name`,
except the supported `advisory.` names described in [Advisory](ADVISORY.md)
and the provisional subject Settings below.
Existing admitted Settings from other mechanisms remain readable in the list.

## HTTP contract

- `GET /settings` returns `schema_version`, every admitted `Setting` in `settings`,
  the effective `palette`, and `inverse` entries for all eleven allowed colours.
  Palette rows have `category`, `color_id`, and `origin`. Inverse rows have
  `color_id`, sorted `categories`, and `status`: `mapped`, `collision`, or
  `unmapped`. No collision is resolved arbitrarily; no unmapped allowed ID is
  omitted.
- `PUT /setting/calendar.color.<category>` accepts
  `{"schema_version":"ubu.orchestrator.setting.v1","value":"2"}`. Values must
  be strings in the same allowed set the startup file loader validates:
  `1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11`. `setting_invalid_color` lists those IDs.
  Missing/unknown schema versions are rejected. Attribution is server-controlled:
  `authority_source: "user"`, provenance `{created_at, authority_source}`, and no
  `source`. Admission uses the ordinary mutation envelope and store path, as the
  existing bootstrap Settings authoring does. Updates retain the ID and creation
  time and advance the version. The response is `schema_version`, `setting_id`,
  and `version` (HTTP 200 for both creation and update).
- `DELETE /setting/calendar.color.<category>` removes the Setting override and
  returns 204 with no body. GET and DELETE require no body/schema-version input.
  An absent override returns 404. A category introduced only by the Setting
  disappears; a seeded category reverts to its fallback. The mutation ledger is
  retained, matching native Preference withdrawal's deletion idiom.

PUT/DELETE serialize against native authoring through the existing locks. PUT
updates the current version without a caller-supplied version condition. Duplicate
pre-existing names are refused for editing rather than choosing an arbitrary row.

## Precedence and lifetime

The precedence, highest first, is:

1. An admitted active Setting named `calendar.color.<category>` — origin `setting`.
2. A startup file override from `UBU_CATEGORY_PALETTE_PATH` — origin `file`.
3. The eleven built-in defaults — origin `default`.

A file value equal to its default still has origin `file`; an equal Setting value
still has origin `setting`. GET reports the effective origin, so the operator can
see why an edited file did not override an admitted Setting.

The validated default/file palette is a **bootstrap seed**, stored in the
orchestrator-owned `category_palette_seed` table at startup. It is not itself a
set of user Setting records. A new startup refreshes this fallback snapshot from
the configured file; it does not erase admitted Settings. A malformed file still
fails before the database is opened, with the existing startup validation error.
Deleting or changing the file after startup does not change the seed for that
process. Editing a file still requires restart.

The effective palette is constructed from the pool for each request, without
caching it on `AppState`. A **Setting colour change needs no restart**: generation,
Calendar preview and capture use the new mapping. Before P1B-42 the effective
palette was loaded once onto `AppState`, so file edits and restart were the only
way to change it.

A Calendar preview re-resolves colours on Static steps using the current palette,
including for an already-stored Plan; the Plan does not need regeneration.
Dynamic events retain the P1B-33 no-colour/completion partition. An already-created
preview remains the reviewed payload: updating Settings does not rewrite that
preview or apply anything to Google. Take a new preview after changing colours.

Setup's Colours card shows both directions, edits through PUT and reverts through
DELETE. It reloads both views after successful writes. For a calendar bootstrap,
read [Calendar bootstrap](CALENDAR_BOOTSTRAP.md) before capture.


## P1B-69: provisional subject registry

UBU-D0291's effective subject vocabulary is the governed set union the
provisional registry. The governed roots operator, project, github, affect and
relationship are code constants, not editable Setting records. The registry
contains only Settings named universe.subject.<root>, with boolean value true.
It is visible as the Subjects list on UniverseState, with provisional roots
marked awaiting ratification. No GET response shape or route is added.

PUT /setting/universe.subject.<root> is an explicit mint, using ordinary Setting
admission and user attribution. The root is lowercase ASCII snake_case, begins
with a letter, contains no dots and is at most 64 bytes. The collection names
and affect are reserved; other governed names and already registered roots
cannot be minted. False, strings and other values are refused; DELETE is the
retirement act. The semantic singular-noun/entity-or-domain rule is stated on
the screen and judged by the operator, never approximated with a language rule.

DELETE removes a provisional Setting only and keeps the existing Setting
withdrawal behavior. Governed roots cannot be deleted. Retiring leaves every
previously authored target untouched and evaluable, while preventing new
writes under that root until it is explicitly minted again. Invalid imported
Settings do not enlarge the effective vocabulary. There is no implicit mint,
ratification, root promotion, migration or new Preference.

Diagnostics are subject_reserved, subject_governed, subject_invalid (root
shape), subject_invalid_value and subject_already_registered. Existing
duplicate-Setting refusal and authoring locks apply. The registry and world
writes share the Task-action lock, so an admitted write cannot race retirement.
