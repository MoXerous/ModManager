# تقرير التدقيق الشامل — القديم ↔ الجديد

> BASE: `4f71f18736c95a1eeb877fe8363f11871118c618` · HEAD: `7ebc828f0f6ff5640f862a31094c8be4f6f34d70`
> مُولَّد من `docs/spec/audit-ledger.json` — لا تُعدِّله يدويًّا.

## الملخّص

- إجمالي الوحدات: **276**
- مطابق: 171
- انحراف موثّق: 73
- انحراف غير موثّق (findings): **25**
- غير قابل للتحقّق: 7

## Findings (مرتّبة بالخطورة)

### A-0001 — medium — behavior (F-001)

- القديم: `app/api/modrinth_client.py::search_projects`
- الجديد: `core/src/providers/modrinth.rs::ModrinthClient::facets`
- method: `code` · reviewer: `deepseek`
- الانحراف: Old search_projects sends facets for project_type, loader, category, extra_categories, game_version, client_side and server_side, plus a caller-chosen index (sort). New SearchQuery (core/src/providers/mod.rs) carries only query/game_version/loader/project_type/limit/offset; facets() emits three facet groups and search_inner hardcodes index=relevance. Category, client-side, server-side and sort filters are absent end to end (SearchQueryDto in src-tauri/src/dto.rs has the same six fields).
- الدليل: Read app/api/modrinth_client.py:217-305: _build_search_facets appends categories:{loader}, categories:{category}, extra categories, versions, client_side and server_side, and search_projects forwards limit/offset/index and query. Read core/src/providers/modrinth.rs:436-448 (facets: only project_type, versions, categories) and 647-653 (query, facets, index="relevance", offset, limit). Read core/src/providers/mod.rs:115-123 (SearchQuery fields) and src-tauri/src/dto.rs:235-251 (SearchQueryDto fields) to confirm the missing filters never reach the provider. Pagination and name search do match; the filter surface is reduced.

### A-0045 — medium — behavior (F-045)

- القديم: `app/services/safe_archive.py::_member_collision_key`
- الجديد: `core/src/safety/inspect.rs::collision_key`
- method: `test` · reviewer: `deepseek`
- الانحراف: The old collision key is `name.rstrip("/").casefold()`; the new key strips one trailing slash, then applies to_lowercase(), then NFC. Rust to_lowercase is not full Unicode casefold, so names whose folding differs from lowercasing (for example the German sharp s folding to "ss", or the "fi" ligature) no longer collide with their expanded form. The source records this as a deliberate shortcut, but feature-parity.json does not, and features.md F-045 is Keep, so the reduced Unicode coverage is an undocumented deviation.
- الدليل: Read app/services/safe_archive.py:188-189 (`_member_collision_key`) and 124-127 (duplicate detection through it). Read core/src/safety/inspect.rs:78-86: the function documents 'lowercase-then-NFC because lowercasing can denormalize' and carries a `shortcut:` comment that Rust to_lowercase is not full Unicode casefold and differs for sharp-s and the fi ligature. Test spec_f044_rejects_casefold_and_nfc_and_slash_collisions proves the case-collision case still fires ('a.TXT' vs 'A.txt' -> Duplicate); the casefold-only cases are the documented gap.

### A-0054 — medium — behavior (F-054)

- القديم: `app/ui/main_window.py::DownloadReviewDialog`
- الجديد: `core/src/resolver/graph/mod.rs::review_candidates`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old review dialog listed optional dependencies as unchecked rows the user could tick. The new app never produces optional rows at all: the production plan path follows only `required` edges, so there is no optional candidate to leave unchecked and no checkbox to leave empty. The pure model that would carry the flags exists and is unit-tested, but nothing in the app calls it.
- الدليل: Read app/ui/main_window.py:1911-1943: the dialog prints selected/required/optional/transitive counts, builds a six-column table whose first column is an 'Include' checkbox, keeps `self._dep_checkboxes`, and syncs parent-gated boxes. Read core/src/resolver/graph/mod.rs:323-441: review_candidates pushes optional deps with selected=false (step 2, lines 419-441) and required ones with selected=true. Grepped core/src, cli/src and src-tauri/src for review_candidates: the only callers are core/src/resolver/graph/tests.rs (spec_f054_optional_unchecked_by_default) and the re-export in resolver/mod.rs; batch.rs:74-84 filters `dependency_type == "required"` and never asks for optionals, so the plan DTO carries no optional row.

### A-0055 — medium — behavior (F-055)

- القديم: `app/services/resolver.py::collect_required_dependencies_for_optional_dependencies`
- الجديد: `core/src/resolver/graph/mod.rs::review_candidates`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old dialog auto-checked the required chain of an optional dependency the moment the user ticked that optional, and the plan then included it. The new app has the rule (review_candidates marks those rows requires_parent and selected) but no optional selection exists to trigger it, so the chain is never pulled in.
- الدليل: Read app/services/resolver.py:617-705: for each optional the function walks its required edges, stamps dependency_parent_project_ids/titles and dependency_requires_selected_parent, and merges by project id; collect_dependency_review_candidates (739-765) is what the dialog consumed. Read core/src/resolver/graph/mod.rs:443-459: step 3 iterates `selected_optional` and pushes each required chain row with requires_parent=true, and 335-370 merges parents. Grepped for selected_optional: only graph/tests.rs (spec_f055_selected_optional_pulls_required_chain) constructs a non-empty slice; batch.rs::resolve_and_plan builds deps itself and never calls review_candidates, so selected_optional is never populated in the app.

### A-0056 — medium — behavior (F-056)

- القديم: `app/services/resolver.py::_preferred_version_if_compatible`
- الجديد: `core/src/resolver/resolve/mod.rs::resolve_project`
- method: `code` · reviewer: `deepseek`
- الانحراف: Both sides honour an imported pin in the order version id, then SHA-1, then version number, and require the pinned version to be compatible. The new code adds an alpha gate the old one did not have: a pin that points at an alpha build is refused when allow_alpha is false, and the resolver then installs the newest stable instead. The production plan path passes allow_alpha = false, so an imported profile pinned to an alpha gets a different file without a message about the substitution.
- الدليل: Read app/services/resolver.py:288-330: id -> get_version, sha1 -> get_versions_from_hashes, number -> unfiltered version list, each accepted when `_version_is_compatible_with_context` passes, and that helper (145-146 -> 126-142) checks only primary file, listed status, game version and loader, never the release channel. Read core/src/resolver/resolve/mod.rs:302-379: the same ladder (get_version_in_project then list scan for the id, hash_lookup for the sha1, list scan for the number) but every branch also requires `params.allow_alpha || v.version_type.to_lowercase() != "alpha"`; core/src/resolver/batch.rs:59 passes allow_alpha: false (no src-tauri caller sets it). Test spec_f056_preferred_pin_order_id_sha1_number covers the order, not the alpha case.

### A-0062 — medium — behavior (F-062)

- القديم: `app/ui/main_window.py::DownloadReviewDialog`
- الجديد: `src/app/pages/InstallPlanDialog.tsx`
- method: `code` · reviewer: `deepseek`
- الانحراف: The dependency-review dialog lost its selection column. The old dialog showed one row per required and optional dependency with an 'Include' checkbox, required rows pre-checked, optional rows unchecked, and the required chain of a ticked optional auto-checked. The new dialog shows the plan's entries and the warnings read-only: no checkbox, no optional row, and no way to include or exclude anything before confirming.
- الدليل: Read app/ui/main_window.py:1893-1953: title 'Review download', a summary line with selected/required/optional/transitive counts, a 6-column table ['Include','Type','Dependency','Version','File','Reason'] with `_dep_checkboxes` and `_sync_parent_gated_dependency_checkboxes`, then a warnings table. Read src/app/pages/InstallPlanDialog.tsx:44-56 (the doc comment claims F-062/F-063) and 214-260: chips for selected/dependencies/downloadable/blocked, a `<ul>` of read-only EntryRow items and a warning list, with a single confirm button; there is no input control and no optional entry because core/src/resolver/batch.rs only follows required edges.

### A-0063 — medium — behavior (F-063)

- القديم: `app/services/batch_install_plan.py::build_batch_install_plan`
- الجديد: `core/src/resolver/plan/mod.rs::build_batch_install_plan`
- method: `test` · reviewer: `deepseek`
- الانحراف: The dry-run preview is equivalent for entries, dedup, counts and warnings, but one confirmation trigger is missing: the old plan also demanded confirmation when `backup_matching_project_files` was set, and the new summary has no such field, so only the four data conditions can raise confirmation. The source comment says the backup flag waits for the caller that owns real backup state, but feature-parity.json does not record the gap and features.md F-063 is Keep.
- الدليل: Read app/services/batch_install_plan.py:46-53 and 112-160: entries are built selected-first with a seen-key set, downloadable/blocked are counted, warnings come from build_download_safety_warnings, and requires_confirmation covers total>1, dependency>0, blocked>0, warnings>0 and the backup flag. Read core/src/resolver/plan/mod.rs:130-172: the same selected-first dedup by (provider, project), the same counts and the same four data conditions, with the doc comment at 165-166 stating the backup flag arrives with the caller that owns real backup state. Tests spec_f063_plan_splits_downloadable_and_blocked, spec_f063_duplicates_dropped_but_counted and spec_f063_requires_confirmation_truth_table cover the four conditions.

### A-0067 — medium — behavior (F-067)

- القديم: `app/services/profile_content_toggle.py`
- الجديد: `core/src/instances/toggle/mod.rs::preview_toggle`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old toggle preview carried a non-blocking 'External/unknown file selected. Continue only if you intentionally want to change this local file marker.' warning (profile_content_toggle.py:143); the new PreviewEntry (core/src/instances/toggle/mod.rs:14-18) exposes only allowed + reason and drops that warning. The rename guards and rollback are unchanged.
- الدليل: Read app/services/profile_content_toggle.py:147-170: disable is only for a top-level .jar in the mods root and renames it to `<name>.disabled`; enable accepts a `.jar.disabled` suffix or a `.jar` inside disabled_mods; both refuse a symlink, a file outside the root, a missing source and an existing destination, with fixed reason strings, and 73-100 renames then rolls back in reverse order on the first failure. Read core/src/instances/toggle/mod.rs:119-201: the same guards in the same order with the same reason strings ('Only top-level active mod files can be disabled by rename.', 'File already appears disabled.', 'Destination already exists; overwrite is not allowed.', ...), the same `.disabled` suffix scheme, and 206+ the same reverse-order rollback with skipped/rolled-back states. Nothing deletes a file. The old preview also carried a non-blocking 'external/unknown file' warning string that the new PreviewEntry (allowed + reason only) does not surface; the rename and its guards are unchanged.

### A-0072 — medium — behavior (F-072)

- القديم: `app/services/mrpack_installer.py::MrpackInstallIssue`
- الجديد: `src-tauri/src/commands/pack.rs::pack_error`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old plan carried a list of issues, each with a stable machine code (manifest_blocker, unsafe_destination, destination_collision, sha1_invalid, sha512_invalid, file_size_invalid, download_url_missing, download_url_rejected, reserved_destination, source_invalid, target_exists, staging_exists, insufficient_disk, ... roughly two dozen in this module) plus severity and path, and the preview rendered them. The new plan DTO has no issue list at all: it carries counts and skip totals, and every index/archive refusal is mapped to one IpcError code (invalidInput) with the variant's Display text as the message. The user still sees a clear sentence, but the named codes and the per-issue preview are gone.
- الدليل: Read app/services/mrpack_installer.py:67-72 (MrpackInstallIssue holds severity, code, message, path) and 155-165 (the plan exposes blockers/warnings/is_installable), plus the code strings at 242-359 and 1549-1700. Read src-tauri/src/commands/pack.rs:83-90 (`fn pack_error(e: &PackError) -> IpcError { IpcError::invalid_input(e) }` with a comment saying one code covers every archive refusal) and 269-318 (plan_dto builds name/version/writes/counts/skip counts only). Read src-tauri/src/dto.rs:1645-1676: ModpackPlanDto has no issues field.

### A-0086 — medium — behavior (F-086)

- القديم: `app/services/local_mod_metadata.py::_read_forge_like_metadata`
- الجديد: `core/src/jar_reader/mod.rs::parse_forge_toml`
- method: `test` · reviewer: `deepseek`
- الانحراف: The old forge metadata reader tolerated a missing mods[0].modId and dict-shaped dependency nodes (local_mod_metadata.py:198-212); the new parser requires mods[0].modId to be a string and only walks array dependency nodes (core/src/jar_reader/mod.rs:290-321). A jar whose mods.toml is malformed but readable by the old app now yields no metadata.
- الدليل: Read app/services/local_mod_metadata.py:198-212: META-INF/mods.toml is parsed with tomllib, `mods[0]` supplies modId/displayName/version/description, the loader is forge, and the first `dependencies` entry with `modId == "minecraft"` contributes its raw `versionRange` to game_versions. Read core/src/jar_reader/mod.rs:290-321: the same `mods[0]` fields through a `toml::Value`, the same first minecraft dependency entry kept verbatim as a range, and the same forge loader. Test spec_f086_forge_keeps_raw_version_range asserts the range is not parsed or rewritten. The new parser requires `mods[0].modId` to be a string and only walks array dependency nodes, where the old tolerated a missing id and a dict-shaped node; both still read every well-formed mods.toml the old app read.

### A-0092 — medium — behavior (F-092)

- القديم: `app/services/instance_detector.py::_detect_metadata`
- الجديد: `src-tauri/src/commands.rs::scan_instance_at`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old detector read launcher metadata to establish a confident target: mmc-pack.json components, modrinth.index.json dependencies and launcher_profiles.json versions produced game version, loader and loader version plus a confidence of confirmed/likely/conflict/unknown. The new app has none of those readers. It only derives loaders by scanning jars and reports loader_source as `jars` or `unknown`; the game version and loader version are not detected at all, so a Prism/MultiMod/Modrinth instance loses its confirmed identity.
- الدليل: Read app/services/instance_detector.py:244-286: `_read_mmc_pack` (296-319), `_read_modrinth_index` (321-346) and `_read_launcher_profiles` (348-366) fill versions/loaders/loader_versions, and the confidence ladder is computed from them; the jar scan at 267-268 runs only when `loaders` is still empty. Read src-tauri/src/commands.rs:378-411: scan_instance_at scans jars, reads the lock, classifies content and calls `detect_loaders(&[], &mods_dir, ...)`, with the comment `Authoritative launchers are empty for now; jar scan fills it (F-092/F-093 sources land with profile support in M2)`. Grepped core/src for mmc-pack.json/launcher_profiles/modrinth.index.json parsing: the only hit is `has_launcher_metadata` in instances/mod.rs:210-214, which tests for the files but never reads them; core/src/instances/registry.rs:644 asserts `row.minecraft_version == None` with the note `detection is F-092/093`.

### A-0101 — medium — behavior (F-101)

- القديم: `app/services/profile_metadata.py::write_profile_json`
- الجديد: `core/src/instances/metadata.rs::write_mirror`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old app rewrote profile.json whenever profile metadata changed: profile materialization, mrpack install, metadata repair, backup and export all called write_profile_json with a freshly built metadata object. The new app has a profile.json mirror and writes it when it creates an instance and when recovery recreates one, but there is no instance update/rename command and the pack-apply, backup and restore paths never refresh it, so the mirror can go stale and the `update path` the feature names does not exist.
- الدليل: Read app/services/profile_metadata.py:160-188: profile_json_path/write_profile_json write a temporary sibling and replace the file, and 191-208 read both the current and the legacy schema. Grepped the old app for callers: profile_materialization.py:63, mrpack_installer.py:633 and 972, profile_metadata_repair.py:139, profile_backup_restore.py:123/301 and profile_export.py:251 all write it. Read core/src/instances/metadata.rs:82-94 (write_mirror) and 125-144 (mirror_of), and grepped its callers: registry.rs:214 is the create path and recovery.rs:423/650/695/730/952 the recovery paths; the complete Tauri instance command list is scan_instance, validate_instance, check_instance_updates, list_instances, plan_create_instance, execute_create_instance, link_instance and open_instance_folder (src-tauri/src/commands.rs:363-918), with no rename/update command.

### A-0102 — medium — behavior (F-102)

- القديم: `app/services/profile_materialization.py::materialize_profile_workspace`
- الجديد: `core/src/instances/registry.rs::plan_create`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old create flow built five standard folders (mods, config, resourcepacks, shaderpacks, saves) plus profile.json and installed_mods.json. The new create builds only mods and config plus profile.json; resourcepacks, shaderpacks and saves are never created for a new instance, so a freshly created instance looks different on disk and the installed_mods.json manifest has no writer on this path.
- الدليل: Read app/services/profile_materialization.py:11 (STANDARD_PROFILE_DIRS is mods, config, resourcepacks, shaderpacks, saves) and 39-65: materialize_profile_workspace preflights, mkdirs the root and every standard folder, then writes profile.json and installed_mods.json. Read core/src/instances/registry.rs:17-22: STANDARD_DIRS is exactly ["mods", "config"] with a comment that saves/resourcepacks/shaderpacks are optional by design, and 122-136/192-215: execute_create creates plan.dirs, writes the mirror and inserts the row, with no installed_mods.json write anywhere in the path.

### A-0106 — medium — behavior (F-106)

- القديم: `app/ui/main_window.py::_choose_existing_create_target_mode`
- الجديد: `core/src/instances/registry.rs::plan_create`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old create dialog preflighted the chosen folder, detected an existing Minecraft instance and offered three explicit choices: Link Existing, Create Here Anyway, or Cancel. The new dialog has a create/link mode toggle but no detection step and no create-anyway option; the Rust plan refuses any non-empty folder, so a user who wants profile.json added to an existing instance is stopped instead of being offered the old choice.
- الدليل: Read app/ui/main_window.py:9642-9656 (_existing_instance_preview_for_create runs preview_profile_folder_import and existing_instance_candidate_reason) and 9657-9683 (_choose_existing_create_target_mode builds Link Existing / Create Here Anyway / Cancel), then 9734-9746 where the create flow branches on that answer. Read core/src/instances/registry.rs:106-120: an existing root is refused as RootIsFile, AlreadyAnInstance when a mirror is readable, or RootNotEmpty when read_dir yields any entry. Read src/app/pages/NewInstanceDialog.tsx:27 (Mode is create or link), 35-42 (the comment claims link-instead), 89-101 (link is a manual action) and 145-163 (the mode toggle); there is no detection call and no create-anyway branch.

### A-0120 — medium — behavior (F-120)

- القديم: `app/ui/main_window.py::UpdatePreviewDialog`
- الجديد: `core/src/updates/mod.rs::check_updates`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old preview was a seventeen-column table that also showed the dependency delta (required/optional/embedded/incompatible counts), the compatibility reason, the safety/touched-files note and the backup plan for each row. The new preview lists only the file, the latest version number and a four-value status; dependency information and a per-row backup plan are not part of the preview or of the install plan, so the user sees less before choosing what to update.
- الدليل: Read app/models.py:78-101: UpdatePreviewResult carries new_required_dependencies, new_optional_dependencies, new_embedded_dependencies, new_incompatible_dependencies, compatibility_reason and release types. Read app/ui/main_window.py:2668-2747: UpdatePreviewDialog builds a 17-column table including Version change, Dependencies, Compatibility reason, Safety / touched files and Backup plan. Read core/src/updates/mod.rs:41-63 and 201-241: UpdateCandidate has filename, current and latest ids/numbers/files/hashes/url/size/game versions and a status, with no dependency or backup field. Read src/app/pages/InstanceDetails.tsx:100-169: the preview renders the filename, the latest version number and the status only, and src/app/pages/UpdateInstallDialog.tsx:180-218 shows the plan entries and the safety contract without a dependency or backup-plan line.

### A-0129 — medium — behavior (F-129)

- القديم: `app/ui/main_window.py::import_installed_scan_matches_to_profile`
- الجديد: `core/src/instances/mod.rs::scan_mods`
- method: `code` · reviewer: `deepseek`
- الانحراف: The scan-and-import flow is absent. The old app scanned an installed folder against the catalogue, showed the matches in a 13-column table and could import the recognized projects into the active profile's saved list; the new app scans an instance only to classify what is already on disk and has no recognized-project table and no import action.
- الدليل: Read app/ui/main_window.py:7084-7138 (scan_installed_folder and _start_installed_scan build a ScanInstalledWorker with the managed manifest location and the target version/loader), 7153-7182 (on_installed_scan_finished opens InstalledScanDialog with allow_profile_import and sync prune) and 7194-7222 (import_installed_scan_matches_to_profile begins an installed import worker). Read core/src/instances/mod.rs:34-40 and 107-208: scan_mods returns LocalContentFact rows for files on disk with classification axes and no provider recognition. Grepped core, src-tauri and src for adopt or import_local: the only hits are InstanceSource::Adopted and the per-root apply-lock adoption in state.rs, neither of which imports a scanned jar.

### A-0144 — medium — behavior (F-144)

- القديم: `app/services/operation_lease.py::OperationLeaseManager`
- الجديد: `src-tauri/src/commands/pack.rs::apply_lock_for`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old app took an OS-level exclusive lease on a .active-operation.lock file next to the operations folder, so a second copy of the application could not run a conflicting operation on the same root and got a Busy answer. The new app only serialises runs inside one process: apply_lock_for returns an in-process tokio Mutex keyed by the canonical instance root, so two copies of the new app pointed at the same instance can still run a pack apply or a backup at the same time. No cross-process lock file exists anywhere in core or src-tauri.
- الدليل: Read app/services/operation_lease.py:11-13 and 66-109 plus 216-218: LOCK_FILENAME is .active-operation.lock, acquire_exclusive refuses when a local or external operation holds the descriptor, and EACCES/EAGAIN/EDEADLK and WinError 33/36 are mapped to OperationLeaseBusy; app/composition.py:87 constructs the manager and app/services/operation_abandonment.py:61 takes it exclusively. Read src-tauri/src/commands/pack.rs:389-395: APPLY_LOCKS is a OnceLock<Mutex<HashMap<PathBuf, Arc<async_runtime::Mutex<()>>>>> and apply_lock_for hands out one in-process mutex per canonical root, with 529-530 and 959 relying on it. Grepped core/src and src-tauri/src for create_new, lock_file, flock and lease: the only hits are instance.lock.json (a data file, not a lease) and the in-process map.

### A-0151 — medium — behavior (F-151)

- القديم: `app/storage/repositories.py::set_setting`
- الجديد: `src/app/preferences.ts::Preferences`
- method: `code` · reviewer: `deepseek`
- الانحراف: The old app persisted every user setting in a settings table (theme, accent, parallel downloads, search limit, resolve workers, download retries, pagination mode, size unit, remember filters, confirm favourite delete, include required dependencies, auto load metadata, auto discover, show instance tools, profile-instance UI mode, last project type, game version, loader, shader platform, sort, client/server side, last download dir and both splitter sizes). The new app has no settings store at all: only four UI flags (theme, locale, numerals, sidebar collapsed) live in localStorage and the rest do not exist, so download concurrency, retry count, search limit and the last search context are not configurable or remembered. The source itself marks this as a shortcut.
- الدليل: Read app/storage/repositories.py:595-615: get_setting, set_setting, get_bool_setting and set_bool_setting read and write the settings table, and app/ui/main_window.py:222-246 declares the 25 SETTING_ constants, with 1284-1306 saving and restoring them. Read src/app/preferences.ts:1-60: Preferences is exactly theme, locale, numerals and sidebarCollapsed, stored under the localStorage key modmanager.preferences.v1 with a shortcut comment saying they live there because three UI flags do not deserve a file yet and that core/src/settings.rs does not exist. Read src/app/pages/SettingsPage.tsx:104-250: the Settings page shows language, theme, digits, the CurseForge key, the five resolved paths and three diagnostics values, and offers no download, search or pagination setting. Grepped core/src and src-tauri/src for SETTING or settings: the only hits are comments and the export settings type, so no core settings module exists.

### A-0170 — medium — behavior (F-170)

- القديم: `app/ui/main_window.py::apply_app_theme`
- الجديد: `src/app/preferences.ts::THEMES`
- method: `code` · reviewer: `deepseek`
- الانحراف: Theme selection matches (system, light, dark, applied before paint, following the OS while system is chosen), but the accent colour choice is gone: the old apply_app_theme took an accent name and built the palette from it, and the old Settings page offered a colour combo, while the new app ships one fixed token palette with no accent picker and no accent setting. feature-parity.json F-170 records the three-way theme but not the missing accent colour.
- الدليل: Read app/ui/main_window.py:1018-1060: apply_app_theme takes theme and accent_name, applies the stored palette or a Fusion light/dark palette, and calls _with_accent_roles and _theme_stylesheet with the accent for every branch. Read app/ui/main_window.py:1118-1124 and 1292-1293: SETTING_THEME and SETTING_ACCENT_COLOR are read and saved from a theme combo and an accent combo. Read src/app/preferences.ts:1-37: THEMES is system, dark, light and Preferences has no accent field. Read src/app/pages/SettingsPage.tsx:109-135: the general card offers language, theme and digits only. Grepped src for accent: every hit is the Tailwind token class or a comment about the fixed accent, and docs/spec/behaviors/settings.md:35 lists SETTING_ACCENT_COLOR as a persisted setting.

### A-0198 — medium — ui (screens.md §3 #5 DownloadReportDialog)

- القديم: `D:\latest_project\app\ui\main_window.py:1527`
- الجديد: `src/app/pages/DownloadsPage.tsx:59`
- method: `browser` · reviewer: `browser`
- الانحراف: The old app produced a download report dialog. The new Downloads page has exactly three tabs (جارٍ / المكتملة / السجلّ), a search field, a clear-history action and no report generation, export or per-batch summary. No R decision records dropping it.
- الدليل: console: 0 errors; palette searched for «تقرير التنزيل» returned «لا شيء يطابق هذا البحث»; the history tab exposes only بحث, امسح السجلّ and five columns; screenshot .opencode/analyses/audit/shots/A-0198.jpeg

### A-0203 — medium — ui (screens.md §3 #10 InstalledScanDialog)

- القديم: `D:\latest_project\app\ui\main_window.py:2502`
- الجديد: `src/app/pages/InstanceDetails.tsx:197`
- method: `browser` · reviewer: `browser`
- الانحراف: F-129 «مسح المودات المحلية + استيراد — جدول 13 عموداً» is Keep in both docs/spec/features.md:208 and docs/spec/feature-parity.md:198, so the 13-column table is a kept behavior, not a permitted reduction. screens.md §6 #2 is only a counts table (pages/dialogs/tabs), and F-097/F-098 cover the classification axes, not column removal; no Defer or Drop decision removes a column either. Yet the old 13-column grid becomes a compact list, and six old columns have no new surface: Version, Project ID, Game versions, Loaders, Matched file and Message — the first two are absent from the rendered row (projectId travels in LocalFactDto but nothing displays it) and the last four are not in the DTO at all. That loss is undocumented. What each row does show: resource type, file name, relative path, the class label with its reason as a tooltip, duplicate identity keys, the hash-mismatch badge and the size.
- الدليل: console: 0 errors; «المحتوى المثبَّت» 3 ملفات listed sodium.jar مُدار 793.3 KB, lithium.jar معطّل 392.7 KB with path disabled_mods/lithium.jar, sodium-extra.jar خارجي 196.3 KB carrying sha1:0f1e2d, and the banner «ملفان أو أكثر بالبيانات نفسها (بصمة SHA-1 واحدة): sha1:0f1e2d»; the row markup (InstanceDetails.tsx:637-701) renders no version, project id, game versions, loaders, matched file or message; screenshot .opencode/analyses/audit/shots/A-0203.jpeg

### A-0206 — medium — ui (screens.md §3 #13 ProfileEditorDialog)

- القديم: `D:\latest_project\app\ui\main_window.py:3036`
- الجديد: `src/app/pages/NewInstanceDialog.tsx:43`
- method: `browser` · reviewer: `browser`
- الانحراف: Create-profile becomes create-instance with the same fields (name, folder, Minecraft version, loader) and the same preview-before-create step, but the old dialog's second mode — «Edit profile» (main_window.py:3036; name, version, loader/platform, folder, notes, entered from ProfileManagerDialog's «Edit» at :4230) — has no equivalent. No command updates an existing instance: generate_handler! (src-tauri/src/lib.rs:127-190) has plan_create_instance / execute_create_instance / link_instance and nothing else, and core/src/db/instances.rs has no update/delete mutator besides insert and mark_scanned (list/get/find_by_path are read-only). The «ربط مجلد قائم» mode takes name and folder only (LinkInstanceRequest), so even a linked instance's detected version/loader cannot be corrected. Medium, not high: creation and linking are complete; only the edit-existing branch is lost.
- الدليل: console: 0 errors; the dialog opened with «إنشاء جديد» selected and showed الاسم, المجلد with استعراض, إصدار ماينكرافت, اللودر and the buttons «معاينة» / «إلغاء»; the instances page offers no edit action; grep over src-tauri/src/lib.rs:127-190 and core/src/db/instances.rs finds no update/rename command; screenshot .opencode/analyses/audit/shots/A-0206.jpeg

### A-0207 — medium — ui (screens.md §3 #14 ProfileInstanceDialog)

- القديم: `D:\latest_project\app\ui\main_window.py:3251`
- الجديد: `src/app/pages/InstanceDetails.tsx:197`
- method: `browser` · reviewer: `browser`
- الانحراف: The old app could export a profile as a launcher instance («Profile as Instance»). The new app treats the instance folder as the unit and has no export-to-launcher action; nothing in R-1..R-11 records the removal.
- الدليل: console: 0 errors; palette searched for «المثيل» matched only the «اذهب إلى / المثيلات» destination and no command; the instance card exposes only افتح المجلد / المحتوى / إخفاء; screenshot .opencode/analyses/audit/shots/A-0207.jpeg

### A-0215 — medium — ui (screens.md §3 #22 ProfileManagerDialog)

- القديم: `D:\latest_project\app\ui\main_window.py:4226`
- الجديد: `src/app/pages/InstancesPage.tsx:149`
- method: `browser` · reviewer: `browser`
- الانحراف: Manage-profiles becomes the instances page, but with most of its actions gone. The old dialog offered New / Edit / Rename / Duplicate / Remove / Import / Export (main_window.py:4226-4276, plus Instance, Health Check, Repair Metadata, Export zip, Import folder, Import zip). The new page has «مثيل جديد» (plan/execute create, with a «ربط مجلد قائم» mode that covers Import folder), open-folder, content and hide only: generate_handler! (src-tauri/src/lib.rs:127-190) has no rename, duplicate, delete or JSON/zip import-export command for an instance, core/src/db/instances.rs has no update/delete mutator besides insert and mark_scanned (list/get/find_by_path are read-only), and core/src/instances has plan_create / execute_create / link_instance. Creating and linking work; managing an existing instance does not. The page-instead-of-dialog shape is a permitted screen reduction (screens.md section 6); the missing actions are not.
- الدليل: console: 0 errors; «المثيلات» rendered «مثيل واحد», buttons مثيل جديد / تحديث, search «ابحث في المثيلات», filters الكل 1 / يحتاج انتباهًا 0, and the card «نسخة تجريبية · 1.21.1 · fabric» with افتح المجلد / المحتوى (and إخفاء once expanded) — no rename, duplicate, remove or import/export; grep of src-tauri/src/lib.rs:127-190 and core/src/{instances,db/instances.rs} confirms those commands do not exist; screenshot .opencode/analyses/audit/shots/A-0215.jpeg

### A-0219 — medium — ui (screens.md §3 Saved mods (R-10))

- القديم: `D:\latest_project\app\ui\main_window.py:11059`
- الجديد: `src/app/CommandPalette.tsx:17`
- method: `browser` · reviewer: `browser`
- الانحراف: R-10 says the saved-mods viewer becomes a side sheet, but no saved-mods surface exists anywhere: the command palette only lists the six destinations and the inspector's Favourites tab is a Discover selection, not the saved mods of an instance. The app has seven saved/favourite i18n keys (browse.inspector.tab.favourites, favouritesCount[_one/_other], favouritesEmpty, removeFavourite, export.savedLabel) and every one belongs to Discover or pack export — none is a per-instance saved list. The old app had a dedicated Saved Mods dialog (main_window.py:11069).
- الدليل: console: 0 errors; palette searched for «محفوظة» returned «لا شيء يطابق هذا البحث»; Object.keys(en.json) filtered by /saved|favourit/ returns exactly 7 keys, all Discover-favourites or export labels; screenshot .opencode/analyses/audit/shots/A-0219.jpeg

## حسب البُعد

| البُعد | الوحدات | مطابق | موثّق | findings | غير مُتحقَّق |
|---|---|---|---|---|---|
| behavior | 181 | 113 | 49 | 19 | 0 |
| cli | 6 | 5 | 1 | 0 | 0 |
| data | 12 | 10 | 2 | 0 | 0 |
| deps | 4 | 3 | 1 | 0 | 0 |
| i18n | 4 | 4 | 0 | 0 | 0 |
| lessons | 10 | 9 | 1 | 0 | 0 |
| packaging | 8 | 0 | 8 | 0 | 0 |
| performance | 7 | 0 | 0 | 0 | 7 |
| safety | 10 | 10 | 0 | 0 | 0 |
| tests | 8 | 8 | 0 | 0 | 0 |
| ui | 26 | 9 | 11 | 6 | 0 |

## غير قابل للتحقّق

- A-0234 (performance): Test-Path benches returned False and Cargo.toml/core/Cargo.toml contain no criterion/divan/bench target; the only startup-adjacent code is db::open (db/mod.rs:198-217), which sets busy_timeout, gates the schema version and asserts WAL before any write; that is design evidence, not a timing.
- A-0235 (performance): No bench target and no measurement file anywhere under core/ or src-tauri/; the 6.2 s figure is the cold run at baseline.md:48 (the warm runs are 2.1 s and the median 2.1 s at :49-51); preferences.applyToDocument (preferences.ts:80-85) settles theme, lang and dir before first paint, which is intended to avoid a flash rather than to cut startup time, so it does not discharge the budget.
- A-0236 (performance): No memory harness in the repo (scripts/ holds only the six audit/parity .mjs builders — build-audit-report, build-feature-parity, build-parity-matrix, build-trace-worklist, check-i18n, test-audit-report — and there is no bench crate); the baseline table is baseline.md:48-51 (194.3 MB median) and performance-budgets.md §5 already warns that the WebView2 memory is not comparable to the Qt process; verdict stays unverifiable rather than claiming a pass.
- A-0237 (performance): baseline.md:48-51 records 111.3 MB median PrivateBytes for the Python app; there is no measurement run, no profiling script and no target/debug build in the repo, so there is nothing to compare.
- A-0238 (performance): instances/mod.rs:135-161 defines the scan roots and core/src/jar_reader/loader.rs reads each jar; no fixture of 354 jars and no timing harness exist in the repo (no benches/, no criterion); baseline.md:59 records 1922.1 ms on the old app.
- A-0239 (performance): jar_reader/cache.rs:1-3 documents the (path,size,mtime) key and cache.rs:23-40 returns a hit without opening the jar; there is no benchmark and no 354-jar fixture, so the 1080 ms → ≤540 ms target remains unmeasured.
- A-0240 (performance): Installed mods live in the installed_mods table (core/src/db/schema.rs:17, written at core/src/db/mod.rs:400); the JSON manifest reader that remains is read_manifest at backup/manifest.rs:238 and manifest_digest at 304, with no timing; baseline.md:62 records 4.2 ms for the old JSON manifest.

## الفهرس (كل الوحدات)

| المعرّف | البُعد | spec | الحكم | الخطورة |
|---|---|---|---|---|
| A-0001 | behavior | F-001 | undocumented-deviation | medium |
| A-0002 | behavior | F-002 | match | info |
| A-0003 | behavior | F-003 | match | info |
| A-0004 | behavior | F-004 | match | info |
| A-0005 | behavior | F-005 | match | info |
| A-0006 | behavior | F-006 | match | info |
| A-0007 | behavior | F-007 | match | info |
| A-0008 | behavior | F-008 | match | info |
| A-0009 | behavior | F-009 | match | info |
| A-0010 | behavior | F-010 | match | info |
| A-0011 | behavior | F-011 | documented-deviation | low |
| A-0012 | behavior | F-012 | documented-deviation | low |
| A-0013 | behavior | F-013 | documented-deviation | low |
| A-0014 | behavior | F-014 | match | info |
| A-0015 | behavior | F-015 | match | info |
| A-0016 | behavior | F-016 | documented-deviation | low |
| A-0017 | behavior | F-017 | match | info |
| A-0018 | behavior | F-018 | match | info |
| A-0019 | behavior | F-019 | match | info |
| A-0020 | behavior | F-020 | match | info |
| A-0021 | behavior | F-021 | match | info |
| A-0022 | behavior | F-022 | match | info |
| A-0023 | behavior | F-023 | match | info |
| A-0024 | behavior | F-024 | documented-deviation | low |
| A-0025 | behavior | F-025 | match | info |
| A-0026 | behavior | F-026 | match | info |
| A-0027 | behavior | F-027 | match | info |
| A-0028 | behavior | F-028 | documented-deviation | low |
| A-0029 | behavior | F-029 | match | info |
| A-0030 | behavior | F-030 | match | info |
| A-0031 | behavior | F-031 | match | info |
| A-0032 | behavior | F-032 | match | info |
| A-0033 | behavior | F-033 | match | info |
| A-0034 | behavior | F-034 | match | info |
| A-0035 | behavior | F-035 | match | info |
| A-0036 | behavior | F-036 | match | info |
| A-0037 | behavior | F-037 | match | info |
| A-0038 | behavior | F-038 | match | info |
| A-0039 | behavior | F-039 | match | info |
| A-0040 | behavior | F-040 | match | info |
| A-0041 | behavior | F-041 | match | info |
| A-0042 | behavior | F-042 | match | info |
| A-0043 | behavior | F-043 | match | info |
| A-0044 | behavior | F-044 | match | info |
| A-0045 | behavior | F-045 | undocumented-deviation | medium |
| A-0046 | behavior | F-046 | match | info |
| A-0047 | behavior | F-047 | match | info |
| A-0048 | behavior | F-048 | match | info |
| A-0049 | behavior | F-049 | match | info |
| A-0050 | behavior | F-050 | match | info |
| A-0051 | behavior | F-051 | match | info |
| A-0052 | behavior | F-052 | match | info |
| A-0053 | behavior | F-053 | match | info |
| A-0054 | behavior | F-054 | undocumented-deviation | medium |
| A-0055 | behavior | F-055 | undocumented-deviation | medium |
| A-0056 | behavior | F-056 | undocumented-deviation | medium |
| A-0057 | behavior | F-057 | match | info |
| A-0058 | behavior | F-058 | match | info |
| A-0059 | behavior | F-059 | documented-deviation | low |
| A-0060 | behavior | F-060 | documented-deviation | low |
| A-0061 | behavior | F-061 | documented-deviation | low |
| A-0062 | behavior | F-062 | undocumented-deviation | medium |
| A-0063 | behavior | F-063 | undocumented-deviation | medium |
| A-0064 | behavior | F-064 | match | info |
| A-0065 | behavior | F-065 | match | info |
| A-0066 | behavior | F-066 | match | info |
| A-0067 | behavior | F-067 | undocumented-deviation | medium |
| A-0068 | behavior | F-068 | match | info |
| A-0069 | behavior | F-069 | match | info |
| A-0070 | behavior | F-070 | match | info |
| A-0071 | behavior | F-071 | match | info |
| A-0072 | behavior | F-072 | undocumented-deviation | medium |
| A-0073 | behavior | F-073 | match | info |
| A-0074 | behavior | F-074 | match | info |
| A-0075 | behavior | F-075 | documented-deviation | low |
| A-0076 | behavior | F-076 | documented-deviation | low |
| A-0077 | behavior | F-077 | documented-deviation | low |
| A-0078 | behavior | F-078 | match | info |
| A-0079 | behavior | F-079 | documented-deviation | low |
| A-0080 | behavior | F-080 | match | info |
| A-0081 | behavior | F-081 | match | info |
| A-0082 | behavior | F-082 | match | info |
| A-0083 | behavior | F-083 | documented-deviation | low |
| A-0084 | behavior | F-084 | documented-deviation | low |
| A-0085 | behavior | F-085 | match | info |
| A-0086 | behavior | F-086 | undocumented-deviation | medium |
| A-0087 | behavior | F-087 | match | info |
| A-0088 | behavior | F-088 | match | info |
| A-0089 | behavior | F-089 | match | info |
| A-0090 | behavior | F-090 | match | info |
| A-0091 | behavior | F-091 | match | info |
| A-0092 | behavior | F-092 | undocumented-deviation | medium |
| A-0093 | behavior | F-093 | match | info |
| A-0094 | behavior | F-094 | match | info |
| A-0095 | behavior | F-095 | documented-deviation | low |
| A-0096 | behavior | F-096 | documented-deviation | low |
| A-0097 | behavior | F-097 | match | info |
| A-0098 | behavior | F-098 | match | info |
| A-0099 | behavior | F-099 | match | info |
| A-0100 | behavior | F-100 | match | info |
| A-0101 | behavior | F-101 | undocumented-deviation | medium |
| A-0102 | behavior | F-102 | undocumented-deviation | medium |
| A-0103 | behavior | F-103 | match | info |
| A-0104 | behavior | F-104 | match | info |
| A-0105 | behavior | F-105 | documented-deviation | low |
| A-0106 | behavior | F-106 | undocumented-deviation | medium |
| A-0107 | behavior | F-107 | match | info |
| A-0108 | behavior | F-108 | match | info |
| A-0109 | behavior | F-109 | match | info |
| A-0110 | behavior | F-110 | match | info |
| A-0111 | behavior | F-111 | match | info |
| A-0112 | behavior | F-112 | match | info |
| A-0113 | behavior | F-113 | documented-deviation | low |
| A-0114 | behavior | F-114 | match | info |
| A-0115 | behavior | F-115 | match | info |
| A-0116 | behavior | F-116 | documented-deviation | low |
| A-0117 | behavior | F-117 | documented-deviation | low |
| A-0118 | behavior | F-118 | match | info |
| A-0119 | behavior | F-119 | documented-deviation | low |
| A-0120 | behavior | F-120 | undocumented-deviation | medium |
| A-0121 | behavior | F-121 | documented-deviation | low |
| A-0122 | behavior | F-122 | match | info |
| A-0123 | behavior | F-123 | documented-deviation | low |
| A-0124 | behavior | F-124 | match | info |
| A-0125 | behavior | F-125 | documented-deviation | low |
| A-0126 | behavior | F-126 | match | info |
| A-0127 | behavior | F-127 | match | info |
| A-0128 | behavior | F-128 | documented-deviation | low |
| A-0129 | behavior | F-129 | undocumented-deviation | medium |
| A-0130 | behavior | F-130 | match | info |
| A-0131 | behavior | F-131 | match | info |
| A-0132 | behavior | F-132 | match | info |
| A-0133 | behavior | F-133 | match | info |
| A-0134 | behavior | F-134 | match | info |
| A-0135 | behavior | F-135 | match | info |
| A-0136 | behavior | F-136 | match | info |
| A-0137 | behavior | F-137 | match | info |
| A-0138 | behavior | F-138 | match | info |
| A-0139 | behavior | F-139 | match | info |
| A-0140 | behavior | F-140 | match | info |
| A-0141 | behavior | F-141 | match | info |
| A-0142 | behavior | F-142 | match | info |
| A-0143 | behavior | F-143 | match | info |
| A-0144 | behavior | F-144 | undocumented-deviation | medium |
| A-0145 | behavior | F-145 | documented-deviation | low |
| A-0146 | behavior | F-146 | match | info |
| A-0147 | behavior | F-147 | documented-deviation | low |
| A-0148 | behavior | F-148 | match | info |
| A-0149 | behavior | F-149 | match | info |
| A-0150 | behavior | F-150 | match | info |
| A-0151 | behavior | F-151 | undocumented-deviation | medium |
| A-0152 | behavior | F-152 | match | info |
| A-0153 | behavior | F-153 | match | info |
| A-0154 | behavior | F-154 | documented-deviation | low |
| A-0155 | behavior | F-155 | documented-deviation | low |
| A-0156 | behavior | F-156 | match | info |
| A-0157 | behavior | F-157 | match | info |
| A-0158 | behavior | F-158 | documented-deviation | low |
| A-0159 | behavior | F-159 | documented-deviation | low |
| A-0160 | behavior | F-160 | documented-deviation | low |
| A-0161 | behavior | F-161 | documented-deviation | low |
| A-0162 | behavior | F-162 | documented-deviation | low |
| A-0163 | behavior | F-163 | documented-deviation | low |
| A-0164 | behavior | F-164 | documented-deviation | low |
| A-0165 | behavior | F-165 | documented-deviation | low |
| A-0166 | behavior | F-166 | documented-deviation | low |
| A-0167 | behavior | F-167 | documented-deviation | low |
| A-0168 | behavior | F-168 | documented-deviation | low |
| A-0169 | behavior | F-169 | match | info |
| A-0170 | behavior | F-170 | undocumented-deviation | medium |
| A-0171 | behavior | F-171 | documented-deviation | low |
| A-0172 | behavior | F-172 | documented-deviation | low |
| A-0173 | behavior | F-173 | match | info |
| A-0174 | behavior | F-174 | match | info |
| A-0175 | behavior | F-175 | documented-deviation | low |
| A-0176 | behavior | F-176 | documented-deviation | low |
| A-0177 | behavior | F-177 | documented-deviation | low |
| A-0178 | behavior | F-178 | documented-deviation | low |
| A-0179 | behavior | F-179 | documented-deviation | low |
| A-0180 | behavior | F-180 | documented-deviation | low |
| A-0181 | behavior | F-181 | match | info |
| A-0182 | data | — | documented-deviation | low |
| A-0183 | data | — | documented-deviation | low |
| A-0184 | data | — | match | info |
| A-0185 | data | — | match | info |
| A-0186 | data | — | match | info |
| A-0187 | data | — | match | info |
| A-0188 | data | — | match | info |
| A-0189 | data | — | match | info |
| A-0190 | data | — | match | info |
| A-0191 | data | — | match | info |
| A-0192 | data | — | match | info |
| A-0193 | data | — | match | info |
| A-0194 | ui | screens.md §3 #1 SettingsDialog | match | info |
| A-0195 | ui | screens.md §3 #2 ActiveProfileSelectDialog | documented-deviation | medium |
| A-0196 | ui | screens.md §3 #3 DataPackWorldTargetDialog | documented-deviation | low |
| A-0197 | ui | screens.md §3 #4 ProfileWorldsDialog | documented-deviation | low |
| A-0198 | ui | screens.md §3 #5 DownloadReportDialog | undocumented-deviation | medium |
| A-0199 | ui | screens.md §3 #6 VersionHistoryDialog | documented-deviation | info |
| A-0200 | ui | screens.md §3 #7 DownloadHistoryDialog | documented-deviation | low |
| A-0201 | ui | screens.md §3 #8 DownloadReviewDialog | match | info |
| A-0202 | ui | screens.md §3 #9 MrpackImportPreviewDialog | match | info |
| A-0203 | ui | screens.md §3 #10 InstalledScanDialog | undocumented-deviation | medium |
| A-0204 | ui | screens.md §3 #11 UpdatePreviewDialog | match | info |
| A-0205 | ui | screens.md §3 #12 BackupManagerDialog | match | info |
| A-0206 | ui | screens.md §3 #13 ProfileEditorDialog | undocumented-deviation | medium |
| A-0207 | ui | screens.md §3 #14 ProfileInstanceDialog | undocumented-deviation | medium |
| A-0208 | ui | screens.md §3 #15 InstanceHealthDialog | documented-deviation | low |
| A-0209 | ui | screens.md §3 #16 ProfileValidationDialog | documented-deviation | low |
| A-0210 | ui | screens.md §3 #17 ProfileExportOptionsDialog | documented-deviation | info |
| A-0211 | ui | screens.md §3 #18 ProfileExportPreviewDialog | documented-deviation | info |
| A-0212 | ui | screens.md §3 #19 ProfileExportZipImportPreviewDialog | match | info |
| A-0213 | ui | screens.md §3 #20 ProfileFolderImportDialog | match | info |
| A-0214 | ui | screens.md §3 #21 ProfileMetadataRepairDialog | documented-deviation | low |
| A-0215 | ui | screens.md §3 #22 ProfileManagerDialog | undocumented-deviation | medium |
| A-0216 | ui | screens.md §3 #23 PreLaunchActivationDialog | documented-deviation | info |
| A-0217 | ui | screens.md §3 #24 OperationCenterDialog | match | info |
| A-0218 | ui | screens.md §3 Target Version Planner (R-9) | match | info |
| A-0219 | ui | screens.md §3 Saved mods (R-10) | undocumented-deviation | medium |
| A-0220 | i18n | src/i18n/en.json + ar.json key parity | match | info |
| A-0221 | i18n | RTL layout | match | info |
| A-0222 | i18n | plural forms (_one/_other and Arabic categories) | match | info |
| A-0223 | i18n | no raw i18n keys or hard-coded UI strings | match | info |
| A-0224 | safety | F-022 verify SHA-1 + SHA-512 | match | info |
| A-0225 | safety | F-034 / F-037 .part then atomic rename | match | info |
| A-0226 | safety | F-041 / F-042 reject path traversal and absolute paths | match | info |
| A-0227 | safety | F-038 / F-044 refuse symlinks and special files | match | info |
| A-0228 | safety | F-043 archive size, count and ratio limits | match | info |
| A-0229 | safety | F-067 / F-108 disable = rename to .jar.disabled | match | info |
| A-0230 | safety | F-019 / F-138 fail-closed redaction | match | info |
| A-0231 | safety | F-131 PRESENT vs ELIGIBLE | match | info |
| A-0232 | safety | F-127 prune preview leaves unknowns | match | info |
| A-0233 | safety | F-080 / F-113 double verification by token and manifest digest | match | info |
| A-0234 | performance | B-1 warm start ≤ 1.0 s | unverifiable | low |
| A-0235 | performance | B-2 cold start ≤ 3.0 s | unverifiable | low |
| A-0236 | performance | B-3 idle WorkingSet ≤ 194.3 MB | unverifiable | low |
| A-0237 | performance | B-4 idle PrivateBytes ≤ 111.3 MB | unverifiable | low |
| A-0238 | performance | B-5 cold scan of 354 jars ≤ 960 ms | unverifiable | low |
| A-0239 | performance | B-6 warm scan of 354 jars ≤ 540 ms | unverifiable | low |
| A-0240 | performance | B-7 manifest read ≤ 10 ms | unverifiable | low |
| A-0241 | packaging | F-176 / foundations.md §7 installers (Windows NSIS/MSI, Linux AppImage/deb/rpm) | documented-deviation | info |
| A-0242 | packaging | foundations.md §7 code signing (SmartScreen) / M4 | documented-deviation | info |
| A-0243 | packaging | foundations.md §7/§10 tauri-plugin-updater / M4 | documented-deviation | info |
| A-0244 | packaging | F-181 legacy data/settings migration (ADR 0003 §3) | documented-deviation | info |
| A-0245 | packaging | release channels / RC promotion (M4 distribution) | documented-deviation | info |
| A-0246 | packaging | package/bundle size audit (M4 distribution) | documented-deviation | info |
| A-0247 | packaging | F-178 packaged diagnostics / F-150-F-151 in-app diagnostics | documented-deviation | info |
| A-0248 | packaging | release manifest and checksums (M4 distribution) | documented-deviation | info |
| A-0249 | deps | core Rust crates (tokio / reqwest / rusqlite / zip / ts-rs) | match | info |
| A-0250 | deps | frontend npm packages (React / Tauri API / Tailwind CSS) | match | info |
| A-0251 | deps | dependency and project licenses | documented-deviation | info |
| A-0252 | deps | dependency maintenance dates and freshness | match | info |
| A-0253 | cli | mm-cli command surface (M1 CLI contract) | documented-deviation | info |
| A-0254 | cli | Tauri command layer / webview-to-core boundary | match | info |
| A-0255 | cli | F-072 typed IpcError codes and i18n translation | match | info |
| A-0256 | cli | plan-then-execute across both clients | match | info |
| A-0257 | cli | dry-run / preview that writes nothing | match | info |
| A-0258 | cli | F-020 progress streaming via Tauri Channels | match | info |
| A-0259 | lessons | lessons.md §1 — dedicated search worker bypasses the global worker lifecycle (shutdown crash) | match | info |
| A-0260 | lessons | lessons.md §2 — stale result from a cancelled task corrupts the new result | match | info |
| A-0261 | lessons | lessons.md §3 — cancellation reported as an empty success | match | info |
| A-0262 | lessons | lessons.md §4 (4.1/4.2/4.3) — scan prune, PRESENT vs ELIGIBLE data-deletion bug, missing file veto | match | info |
| A-0263 | lessons | lessons.md §5 — created profile never becomes the active scan/import context | match | info |
| A-0264 | lessons | lessons.md §6 — a diagnostic that checks everything slows the common-target path | match | info |
| A-0265 | lessons | lessons.md §7 — packaging-stage failures read as general architecture patterns | match | info |
| A-0266 | lessons | lessons.md §8 — painful areas: licensing, Linux packaging, Windows long paths, 13k-line main window | documented-deviation | info |
| A-0267 | lessons | lessons.md §9 — checklist L-1..L-13, especially L-10 (broad assertRaises hides the failing call) | match | info |
| A-0268 | lessons | lessons.md §10 — mark what is unverified instead of guessing | match | info |
| A-0269 | tests | tests-to-port.md §3-A download & hash verification (25 old tests) | match | info |
| A-0270 | tests | tests-to-port.md §3-A provider retry/rate-limit resilience (16 old tests) | match | info |
| A-0271 | tests | tests-to-port.md §3-A archive safety (12 old tests) | match | info |
| A-0272 | tests | tests-to-port.md §3-A migrations and future-schema gate (36 old tests) | match | info |
| A-0273 | tests | tests-to-port.md §3-A operation journal (34 old tests) | match | info |
| A-0274 | tests | tests-to-port.md §3-A resolver contracts (15 old tests) | match | info |
| A-0275 | tests | tests-to-port.md §3-A .mrpack installer (31 old tests) | match | info |
| A-0276 | tests | tests-to-port.md §3-A log sanitizer + content inventory (18+21 old tests) and the TS/i18n state slice | match | info |
