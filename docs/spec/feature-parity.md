# مصفوفة تكافؤ الميزات — القديم ↔ نسخة الرست

> **مولَّدة آليًّا**: `node scripts/build-feature-parity.mjs` من `docs/spec/features.md`
> (سلوكُ التطبيق القديم) و`docs/spec/parity.md` (تتبّعُ الاختبارات) و`docs/spec/feature-parity.json`
> (أحكامٌ مُراجَعةٌ بأدلّة، مراجَعٌ مرّةً ثانية من وكيلٍ مستقلّ).

> **درجاتُ الدليل**: «مُراجَع» = حكمٌ على السلوك بأدلّةٍ ملموسة · «بتتبّعِ اختبار» = اختبارٌ يحمل
> المعرّف (دليلٌ على وجود الشيفرة، لا على مطابقتها للسلوك) · «قرارُ المواصفة» = `Defer`/`Drop`.

## الملخّص

- **181** بندًا في المواصفات · **152** منها `Keep` · 16 `Defer` · 13 `Drop`.

| الدرجة | بنود `Keep` |
|---|---|
| حاضر — مُراجَع | 34 |
| حاضر — بتتبّعِ اختبار | 97 |
| جزئي — مُراجَع | 8 |
| مفقود — مُراجَع | 10 |
| خارج النطاق — مُراجَع | 3 |
| قيد المراجعة (لا دليل) | 0 |

**18** بندًا `Keep` يحتاج عملًا في نسخة الرست (مفقودٌ أو جزئي).

## ما ينقص نسخة الرست — قائمة العمل

| المعرّف | السلوك | المجال | الحالة | ما ينقص بالضبط |
|---|---|---|---|---|
| **F-003** | `get_best_version` | providers | جزئي — مُراجَع | No release-channel ladder: the new select_best_version takes the first compatible version in provider order and only skips alpha unless allow_alpha; the old get_best_version iterated release > beta (allow_beta default true) > alpha and picked the newest of that channel by date_published, with a newest-of-all fallback. So a newer beta now beats an older stable, there is no allow_beta switch, and no sort by date_published. |
| **F-009** | **قاعدة السنة الدوّارة** | providers | جزئي — مُراجَع | The rotating +/-50-year rule for RFC-850 dates is not implemented: httpdate 1.0.3 maps two-digit years into a fixed 1970-2069 window (crate docs line 51-52, date.rs), so 'Fri, 11-Sep-76' parses as 1976 (past => 0 s wait) where the old code's rolling window computes 2076 (future => the >10s fail-fast rule fires). Header order and strict numeric parsing match the old code; no test anywhere feeds an HTTP-date Retry-After. |
| **F-015** | حالة `downloadUrl: null` عند CurseForge | providers | جزئي — مُراجَع | The data level is handled (null downloadUrl maps to "", files stay listed, plan entry blocked with the exact reason), but the spec's visible behaviour — opening the file's page in the browser — has no counterpart: nothing builds a CurseForge file/project page URL, and the only opener usages (commands.rs::open_app_folder, commands.rs::open_instance_folder) call opener().open_path on local folders, never a web URL. |
| **F-095** | التحقق من البروفايل (30 فحصاً) | instances | مفقود — مُراجَع | No read-only profile validation exists: no severity ladder, no 30-check report, no overall-status derivation, no IPC command or screen; the only trace is a comment in registry.rs about optional standard dirs. features.md keeps F-095 (M2) while InstanceDetails.tsx:115 declares validate/repair dropped on purpose — the conflict the reviewer flagged is real and needs an explicit decision, but per features.md it is a gap, not a by-decision. |
| **F-097** | التصنيف: مدار/خارجي/معطّل/مكرر/غير متوافق | instances | جزئي — مُراجَع | تحليلُ نطاقاتِ إصدارات ماينكرافت غيرُ منفَّذ ⇒ ملفٌّ نطاقُه مسجَّل لا يُحكم عليه (Unknown مع سبب)، وصنفُ «غير المتوافق» لا يُشتقّ من النطاقات. شريحةٌ لاحقة (عزلُها خلف is_exact_version)، مع تصحيح اختبار range-only عندها. |
| **F-121** | **إعادة تحقق قبل التنفيذ** | updates | جزئي — مُراجَع | The stale-plan gate exists only in the flows that exist: pack apply re-hashes the archive (PackChanged) and export compares expectedLockSha512 (InstanceChanged), both with tests. The CLI update is the only update-install path and it reads the lock once, prints rows, confirms and swaps without re-reading the lock, the instance target or the folder — no ticket, no context comparison. The GUI has no update-install command at all (only checkInstanceUpdates). docs/todo.md records F-121..F-124 as deliberately deferred to a later milestone, but features.md keeps them, so they stay gaps. |
| **F-122** | عقد السلامة (5 بنود) | updates | مفقود — مُراجَع | The five-item safety contract copy (timestamped backup, no permanent delete, unknown files untouched, changed-since-preview update skipped, never overwrite an unlinked existing file) is absent everywhere; the only confirmation is confirm_forced's count-only prompt 'Apply N change(s)? [y/N]'. |
| **F-123** | **نسخ احتياطي Yes/No/Cancel** | updates | مفقود — مُراجَع | No Yes/No/Cancel backup decision exists; cli update.rs unconditionally renames each replaced old jar into the shared backup dir before placing the new one, with no prompt and no whole-profile backup path the Yes branch could call. |
| **F-124** | تحديث بالتنزيل فقط | updates | مفقود — مُراجَع | No download-only mode: cli update.rs downloads straight into mods/ (line 122 targets mods.join(filename); the .part gives per-file atomicity but at the live destination) and there is no separate staging folder, no 'touch nothing installed' guarantee, and no journaled update operation to recover the swap. |
| **F-125** | مخطّط الإصدار المستهدف | updates | مفقود — مُراجَع | No target-version planner: core/src/updates/mod.rs implements only the read-only F-120 preview at the instance's current game version/loader; there is no per-mod current->target table, no game_version/loader write-back, and grep for target_version/planner across core, CLI and bridge finds only the modpack planner comments. |
| **F-126** | اقتراح هدف مشترك | updates | مفقود — مُراجَع | No common-target suggestion: no intersection of game_versions across projects, no loader-verified best-upgrade candidate, no sticky pinned targets; grep for suggest/common_target/target_version returns nothing relevant in core, CLI or src. |
| **F-127** | **تنظيف الهدف يترك المجهول** | updates | مفقود — مُراجَع | No target cleanup step exists, so nothing keeps and reports unknown local files during one; the nearest preserved-unknown behaviours are the removal flow's typed REMOVE gate and the pack apply's own-destinations-only rule. |
| **F-130** | **قصّ الميتافيزقا فقط** | updates | مفقود — مُراجَع | No metadata-only prune/reconcile: lock entries are dropped only by explicit remove; scan_instance returns facts plus lock entries without reconciling them; grep for prune/reconcile in core finds one unrelated comment. No plan, no mandatory dry-run, no typed report. |
| **F-131** | **التمييز «موجود» عن «مؤهَّل»** | updates | جزئي — مُراجَع | The present-vs-eligible separation is structural (LocalContentFact per file with Option project_id vs LockEntry, joined by filename in the bridge) and cannot silently collapse, but no consumer compares the two sets (no prune, no import reconcile) and no spec_f131_* test exists anywhere. |
| **F-132** | كاش لكل تشغيل في المخطّط | updates | جزئي — مُراجَع | The per-run cache is exercised by a planner, not only the resolver: core/src/resolver/batch.rs creates RunCache::new(true) and feeds it through resolve_dependency for the whole batch install plan (F-063) before build_batch_install_plan runs, so the 'per-run cache inside a planning flow' behaviour is present for the planner that exists. What is missing is the target-version planner itself (F-125, deliberately deferred), which this same cache would serve; no spec_f132 test pins the planner-scope behaviour. |
| **F-145** | مركز العمليات (قراءة فقط) | operations | جزئي — مُراجَع | No unified operations view: only two per-kind recovery surfaces exist (list_pending_creates for instance creates, pending_pack_recoveries for pack applies), each with its own banner; download-batch records are never surfaced in any recovery UI (no IPC command lists unfinished DownloadBatch operations), there is no alerts table with Refresh, and no assessor for an arbitrary record. |
| **F-157** | `export_manifest.json` | - | مفقود — مُراجَع | No in-archive policy record: write_export_zip emits only modrinth.index.json plus overrides (no export_manifest.json); ExcludedEntry/SkipReason decisions are shown in the preview UI and then discarded, so the artefact cannot be machine-audited after it leaves the app. |
| **F-158** | **استيراد ميتا فقط بلا استخراج** | - | مفقود — مُراجَع | No archive import of the app's own export: the only import commands are the legacy old-home scan/plan/execute (F-181, reads app.sqlite3 + profile.json without extraction) and the .mrpack full install (F-068); there is no way to pick a profile-export zip, preview its manifest metadata-only, schema-check it, or create an instance from it. |

## التفاصيل

### providers

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-001** | بحث Modrinth مع فلاتر | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-002** | جلب مشروع/إصدار | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-003** | `get_best_version` | Keep | M1 | جزئي — مُراجَع | E:\ModManager\core\src\resolver\graph\mod.rs::select_best_version (lines 159-171: find() first compatible, alpha skip only) · D:\latest_project\app\api\modrinth_client.py::get_best_version (lines 422-446: allowed_types ladder + sorted(date_published)) |
| **F-004** | بحث بالهاش (SHA-1) | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-005** | بحث التحديثات دفعة واحدة | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/modrinth.rs، src-tauri/src/progress.rs · كود: core/src/providers/mod.rs |
| **F-006** | تحميل بالهاش | Keep | M1 | حاضر — مُراجَع | core/src/providers/mod.rs::Provider::get_versions_by_hashes · core/src/providers/modrinth.rs::get_versions_by_hashes · core/src/resolver/resolve/mod.rs::hash_lookup · core/src/modpack/exec.rs::PlanExecutor::resolved_urls |
| **F-007** | معالجة حد المعدل 429 | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-008** | **قاعدة الانتظار** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-009** | **قاعدة السنة الدوّارة** | Keep | M1 | جزئي — مُراجَع | E:\ModManager\core\src\providers\modrinth.rs::server_delay_secs (line 210: httpdate::parse_http_date, duration_since unwrap_or(0.0)) · D:\latest_project\app\api\modrinth_client.py::_server_retry_delay (lines 68-78: rolling 50-year window on '-' dates) · C:\Users\LOQ\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\httpdate-1.0.3\src\lib.rs (lines 50-52: 'Two digit years are mapped to dates between 1970 and 2069') |
| **F-010** | إعادة المحاولة (2 محاولات) | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/modrinth.rs |
| **F-011** | تدهور التوافق عند 400 | Defer | M1 | مؤجَّل (قرارُ المواصفة) | — |
| **F-012** | تحديد معدل من جهة العميل | Defer | M1 | مؤجَّل (قرارُ المواصفة) | — |
| **F-013** | تخزين مؤقت لنتائج البحث | Defer | M1 | مؤجَّل (قرارُ المواصفة) | — |
| **F-014** | **مزوّد CurseForge** | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/curseforge/mod.rs، core/src/providers/curseforge/secret.rs (+3) · كود: src-tauri/src/dto.rs، src-tauri/src/providers/tests.rs |
| **F-015** | حالة `downloadUrl: null` عند CurseForge | Keep | M3 | جزئي — مُراجَع | E:\ModManager\core\src\providers\curseforge\mod.rs::map_file · E:\ModManager\src-tauri\src\commands.rs::open_app_folder (line 192: opener().open_path) · E:\ModManager\src-tauri\src\commands.rs::open_instance_folder (line 827: opener().open_path) |
| **F-016** | بصمة CurseForge (`fileFingerprint`) | Defer | M3 | مؤجَّل (قرارُ المواصفة) | — |
| **F-017** | رابط تنزيل فارغ | Keep | M1 | حاضر — بتتبّعِ اختبار | كود: core/src/providers/curseforge/mod.rs، core/src/providers/mod.rs (+3) · اختبار (ذكر): src-tauri/src/commands/tests/browse_tests.rs · وثائق: docs/adr/0004-curseforge-provider.md |
| **F-018** | `User-Agent` وصفي **بمعلومات تواصل** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/lib.rs · كود: core/src/lib.rs |
| **F-019** | **رفض تسريب جسم الاستجابة** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/providers/modrinth.rs، src-tauri/src/commands/tests.rs · كود: core/src/db/download_history.rs، core/src/resolver/resolve/mod.rs (+1) |
### downloads

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-020** | تنزيل جماعي بتقدّم مجمَّع | Keep | M1 | حاضر — مُراجَع | core/src/downloader/batch.rs::execute_batch · src-tauri/src/progress.rs::batch_to_dto · src/app/pages/DownloadsPage.tsx::BatchBar |
| **F-021** | ملف `.part` ثم إعادة تسمية ذرّية | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs، core/src/safety/atomic.rs · كود: core/src/safety/atomic.rs |
| **F-022** | التحقق SHA-1 + SHA-512 | Keep | M1 | حاضر — مُراجَع | core/src/downloader/client.rs::Downloader::verify_streamed · core/src/downloader/client.rs::Downloader::verify_and_commit · core/src/downloader/client.rs::hash_file · core/src/downloader/client.rs::check_file · core/src/downloader/client.rs::remove_part_quiet |
| **F-023** | فحص المساحة قبل الطلب | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs |
| **F-024** | **عتبة المساحة دقيقة بلا هامش** | Defer | M1 | مؤجَّل (قرارُ المواصفة) | — |
| **F-025** | الاستئناف عبر `Range` | Keep | M1 | حاضر — مُراجَع | core/src/downloader/client.rs::Downloader::try_once · core/src/safety/atomic.rs::part_path · core/src/downloader/client.rs::Downloader::verify_and_commit |
| **F-026** | **إعادة بداية إن لم يدعم الخادم `Range`** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs |
| **F-027** | إعادة المحاولة مع `Retry-After` + backoff | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs، core/src/downloader/retry.rs |
| **F-028** | **عشوائية في backoff** | Defer | M1 | مؤجَّل (قرارُ المواصفة) | — |
| **F-029** | التحقق من كل وجهة إعادة توجيه | Keep | M1 | حاضر — مُراجَع | core/src/downloader/client.rs::Downloader::try_once (manual redirect loop, validate_url before every hop) · core/src/downloader/client.rs::validate_url · core/src/downloader/client.rs::Downloader::with_policy (redirect Policy::none) |
| **F-030** | رفض استجابة HTML | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs |
| **F-031** | فرض HTTPS + قائمة بيضاء للمضيفين | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: cli/src/commands/mod.rs، core/src/downloader/client.rs |
| **F-032** | تخطّي ملف موجود متطابق الهاش | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs، core/src/modpack/exec.rs |
| **F-033** | **عدم الكتابة فوق ملف مستخدم** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs |
| **F-034** | حذف `.part` عند فشل التحقق | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs، core/src/safety/atomic.rs · وثائق: docs/plans/2026-10-15-master-plan.md |
| **F-035** | الإيقاف المؤقت / الاستئناف / الإلغاء | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs · وثائق: docs/plans/2026-10-15-master-plan.md |
| **F-036** | **الإلغاء حالة مستقلة لا نجاح** | Keep | M1 | حاضر — مُراجَع | core/src/downloader/batch.rs::execute_batch · core/src/downloader/client.rs::download · src-tauri/src/commands.rs::execute_install_with · src-tauri/src/commands.rs::install_batch_error · src-tauri/src/state.rs::begin_install · src-tauri/src/dto.rs::ErrorCode::Cancelled |
| **F-037** | أطوار معلنة قبل كل عمل | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/exec_tests.rs، src-tauri/src/commands/channel_tests.rs (+1) · اختبار (ذكر): src-tauri/src/commands/channel_tests.rs |
| **F-038** | حارس سباق الروابط الرمزية | Keep | M1 | حاضر — مُراجَع | core/src/safety/guard.rs::open_no_follow · core/src/safety/guard.rs::handle_is_link_or_reparse · core/src/safety/guard.rs::link_in_ancestors · core/src/downloader/client.rs::open_part_handle · core/src/safety/atomic.rs::commit_part · src-tauri/src/dto.rs::ErrorCode::LinkOrReparse |
| **F-039** | التنزيل بالتوازي | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs · كود: core/src/downloader/client.rs، core/src/downloader/mod.rs (+1) |
| **F-040** | صفحة التنزيلات (3 تبويبات) | Keep | M2 | حاضر — مُراجَع | src/app/pages/DownloadsPage.tsx (TABS = live | completed | history, role=tablist) · src/App.tsx (downloads route) · src/app/Shell.tsx::PAGE_IDS · src-tauri/src/commands.rs::list_download_history / clear_download_history · core/src/db/download_history.rs |
### archive

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-041** | رفض اجتياز المسار | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/safety/extract.rs، core/src/safety/path.rs · كود: core/src/safety/extract.rs، core/src/safety/path.rs |
| **F-042** | رفض المسار المطلق وأقراص المنصّة | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/client.rs، core/src/safety/extract.rs (+1) · كود: core/src/downloader/client.rs، core/src/safety/inspect.rs |
| **F-043** | حدود الحجم/العدد/نسبة الضغط | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/safety/extract.rs، core/src/safety/inspect.rs · كود: core/src/safety/inspect.rs، core/src/safety/path.rs |
| **F-044** | رفض الروابط الرمزية والملفات الخاصة | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/safety/extract.rs، core/src/safety/inspect.rs · كود: core/src/safety/extract.rs، core/src/safety/inspect.rs |
| **F-045** | كشف تصادمات Unicode والحالة | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/safety/inspect.rs |
| **F-046** | رفض الأعضاء المشفّرة | Keep | M1 | حاضر — بتتبّعِ اختبار | كود: core/src/safety/inspect.rs |
| **F-047** | رفض الأسماء المحجوزة في ويندوز | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/safety/extract.rs، core/src/safety/path.rs · كود: core/src/safety/extract.rs، core/src/safety/path.rs |
| **F-048** | قراءة عضو بسقف | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/safety/inspect.rs · كود: core/src/safety/inspect.rs |
| **F-049** | رفض الأرشيف الفارغ | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/safety/extract.rs · كود: core/src/safety/extract.rs |
### resolver

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-050** | اختيار الإصدار بالإصدار+Lودر | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs |
| **F-051** | **لودر صارم للمودات** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-052** | تدرّج المنصّات لغير المودات | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/graph/tests.rs |
| **F-053** | التبعيات المطلوبة تضاف تلقائياً | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs |
| **F-054** | التبعيات الاختيارية **غير محددة افتراضياً** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/graph/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-055** | **سلسلة المطلوبة للأختيارية** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/graph/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-056** | احترام التثبيت (id/هاش/رقم) | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-057** | التثبيت غير المتوافق ⇒ أحدث | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/graph/tests.rs، core/src/resolver/resolve/tests.rs · كود: core/src/resolver/graph/mod.rs |
| **F-058** | كاش لكل تشغيل | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/resolve/tests.rs · كود: core/src/resolver/cache.rs |
| **F-059** | تشخيص الكاش | Drop | — | مُستبعد (قرارُ المواصفة) | — |
| **F-060** | **كشف تعارض الإصدارات** | Defer | M3 | مؤجَّل (قرارُ المواصفة) | — |
| **F-061** | **كشف الدورات** | Defer | M3 | مؤجَّل (قرارُ المواصفة) | — |
| **F-062** | حوار مراجعة التبعيات | Keep | M2 | حاضر — بتتبّعِ اختبار | كود: src/app/pages/InstallPlanDialog.tsx |
| **F-063** | **خطة التثبيت الجافة** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/downloader/batch_tests.rs، core/src/resolver/plan/tests.rs · كود: core/src/downloader/batch.rs، core/src/resolver/plan/mod.rs (+6) · اختبار (ذكر): core/src/downloader/batch_tests.rs |
| **F-064** | تحذيرات السلامة المحلية | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/plan/tests.rs |
| **F-065** | تحذير الوجهة الموجودة | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/plan/tests.rs، src-tauri/src/commands/tests.rs · كود: core/src/resolver/batch.rs، core/src/resolver/plan/mod.rs (+1) |
| **F-066** | **لا شيء يحجب في `safety_checks`** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/resolver/plan/tests.rs · كود: core/src/resolver/plan/mod.rs، src-tauri/src/dto.rs (+1) |
| **F-067** | تعطيل = إعادة تسمية `.jar.disabled` | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/tests.rs، core/src/instances/toggle/tests.rs (+1) · كود: core/src/instances/mod.rs، core/src/instances/toggle/mod.rs |
### modpacks

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-068** | استيراد `.mrpack` | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/exec_tests.rs، src-tauri/src/commands/pack_tests.rs · وثائق: docs/adr/0005-pack-url-policy.md |
| **F-069** | **معاينة غير مُعدِّلة** | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/archive_tests.rs، core/src/modpack/exec_tests.rs (+1) |
| **F-070** | تحقق صارم من الفهرس (9 فئات حواجب) | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/archive_tests.rs، core/src/modpack/tests.rs |
| **F-071** | التحقق من هاش الملفات البعيدة | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/exec_tests.rs |
| **F-072** | **26 رمز خطأ مسمّى** | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/archive_tests.rs، core/src/modpack/exec_tests.rs (+1) |
| **F-073** | أولوية `overrides` < `client-overrides` | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/archive_tests.rs · كود: core/src/modpack/archive.rs، core/src/modpack/mrpack.rs |
| **F-074** | تخطّي محتوى الخادم | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/archive_tests.rs · كود: core/src/modpack/archive.rs، core/src/modpack/mrpack.rs (+3) |
| **F-075** | **نشر ذرّي بلا استبدال** | Keep | M3 | خارج النطاق — مُراجَع | core/src/modpack/exec.rs::PlanExecutor::apply (writes into a pre-existing registered instance in place) · core/src/modpack/exec.rs::apply_download (two-phase: `.part` → hash check → `commit_part` rename) · core/src/safety/atomic.rs::commit_part / ::write_part / ::write_atomic · docs/todo.md (design decision recorded: in-place apply, no backup, resume instead of rollback — explicitly divergent from the old atomic directory publish) |
| **F-076** | **تراجع على أي فشل ما قبل الالتزام** | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/journal/store/tests.rs، core/src/journal/tests.rs (+4) · كود: core/src/journal/mod.rs، core/src/journal/store/mod.rs (+8) · اختبار (ذكر): core/src/modpack/exec_tests.rs، core/src/modpack/recovery_tests.rs (+1) |
| **F-077** | التراجع **يرفض بلا إيصال مطابق** | Keep | M3 | خارج النطاق — مُراجَع | core/src/modpack/recovery.rs (module header: "Resume, never rollback") · core/src/modpack/recovery.rs::pending_pack_applies (verifies the archive SHA-512 against the record's pack block before offering resume) · core/src/modpack/exec.rs::clear_interrupted_parts (removes only `.part` files the journal names, never a foreign one) |
| **F-078** | احتياطي روابط التنزيل | Keep | M3 | حاضر — مُراجَع | core/src/modpack/exec.rs::PlanExecutor::candidate_urls (provider URL first, then the pack's own `downloads[]` for the matching SHA-512, deduplicated) · core/src/modpack/exec.rs::PlanExecutor::apply_download (per-URL fresh staging, `continue` to the next candidate on transport error or hash mismatch, last error returned) · core/src/modpack/exec.rs::PlanExecutor::resolved_urls (one `get_versions_by_hashes` round trip for the whole plan) · docs/adr/0005-pack-url-policy.md |
| **F-079** | قائمة بيضاء لمضيفين `.mrpack` | Keep | M3 | خارج النطاق — مُراجَع | core/src/downloader/client.rs::validate_url (https-only, credentials rejected, loopback only under the test flag) · core/src/modpack/exec.rs::apply_download (SHA-512 of the index is the gate before the rename) · docs/adr/0005-pack-url-policy.md (rejected the host allowlist explicitly: trust the hash, not the host) |
| **F-080** | كشف التلاعب بعد المعاينة | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/recovery_tests.rs · كود: src-tauri/src/commands/pack.rs · اختبار (ذكر): core/src/modpack/recovery_tests.rs |
| **F-081** | تصدير `.mrpack` | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/export_tests.rs، src-tauri/src/commands/export_tests.rs · كود: core/src/modpack/export.rs، src-tauri/src/commands/pack.rs (+3) · اختبار (ذكر): core/src/modpack/export_tests.rs |
| **F-082** | تصدير **مع** overrides | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/export_tests.rs، src-tauri/src/commands/export_tests.rs · كود: core/src/modpack/export.rs، src-tauri/src/commands/pack.rs (+5) · اختبار (ذكر): core/src/modpack/export_tests.rs · وثائق: docs/adr/0003-scope-decisions.md |
| **F-083** | استيراد/تصدير CurseForge zip | Keep | M3 | حاضر — بتتبّعِ اختبار | كود: core/src/modpack/export.rs · وثائق: docs/adr/0005-pack-url-policy.md |
| **F-084** | packwiz | Defer | later | مؤجَّل (قرارُ المواصفة) | — |
### jar

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-085** | قراءة `fabric.mod.json` | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/jar_reader/tests.rs |
| **F-086** | قراءة `META-INF/mods.toml` (Forge) | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/jar_reader/tests.rs |
| **F-087** | قراءة `quilt.mod.json` | Keep | M3 | حاضر — مُراجَع | core/src/jar_reader/mod.rs::parse_quilt · core/src/jar_reader/mod.rs::quilt_minecraft_versions · core/src/jar_reader/mod.rs::read_local_mod_metadata · core/src/jar_reader/loader.rs::FIRST_WINS |
| **F-088** | قراءة `META-INF/neoforge.mods.toml` | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/jar_reader/tests.rs |
| **F-089** | **أولوية صارمة** (أول ملف يطابق) | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/jar_reader/tests.rs |
| **F-090** | تصنيف نوع المورد | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/jar_reader/tests.rs · كود: core/src/instances/mod.rs |
| **F-091** | **الميتافيزقا لا تُثبت هوية** | Keep | M3 | حاضر — بتتبّعِ اختبار | كود: core/src/jar_reader/mod.rs |
### instances

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-092** | كشف الـloader من بيانات المشغّل | Keep | M3 | حاضر — بتتبّعِ اختبار | كود: core/src/instances/registry.rs، src-tauri/src/commands.rs (+2) |
| **F-093** | **فحص الـjars بديل فقط** | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/jar_reader/tests.rs · كود: src-tauri/src/commands.rs، src-tauri/src/dto.rs (+1) |
| **F-094** | كشف تخطيط Prism/MultiMC | Keep | M3 | حاضر — مُراجَع | core/src/instances/mod.rs::detect_instance_roots (casefolded `.minecraft`/`minecraft` child + parent metadata ⇒ parent is the instance root) · core/src/instances/mod.rs::has_launcher_metadata (mmc-pack.json | instance.cfg | modrinth.index.json) · core/src/instances/registry.rs::link_instance · core/src/legacy/import.rs (same detection when importing an old profile root) |
| **F-095** | التحقق من البروفايل (30 فحصاً) | Keep | M2 | مفقود — مُراجَع | E:\ModManager\src\app\pages\InstanceDetails.tsx (line 115: 'Validate, repair, backup and restore are gone on purpose') · grep 'validate|severity' in core/src → no profile-validation code · E:\ModManager\src-tauri\src\lib.rs::generate_handler (no validate command) |
| **F-096** | فحص صحة النسخة | Defer | M2 | مؤجَّل (قرارُ المواصفة) | — |
| **F-097** | التصنيف: مدار/خارجي/معطّل/مكرر/غير متوافق | Keep | M3 | جزئي — مُراجَع | core/src/instances/content.rs::classify_content · core/src/instances/content.rs::compatibility · core/src/instances/lock.rs::LockEntry.game_versions · core/src/instances/record.rs::InstalledRecord.game_versions · src/app/pages/InstanceDetails.tsx::classTitle |
| **F-098** | **المحاور الأربعة متعامدة** | Keep | M3 | حاضر — مُراجَع | core/src/instances/content.rs::LocalContentFact · core/src/instances/content.rs::ContentClass::class |
| **F-099** | كشف التكرار (SHA-1 + هوية المشروع) | Keep | M3 | حاضر — مُراجَع | core/src/instances/content.rs::duplicate_keys · core/src/instances/content.rs::DuplicateKey |
| **F-100** | كشف تعارض الهاش | Keep | M3 | حاضر — مُراجَع | core/src/instances/content.rs::hashes_differ · core/src/instances/content.rs::ContentWarning::ManagedHashMismatch |
| **F-101** | مسار تحديث ميتافيزقا البروفايل | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/instances.rs، core/src/instances/metadata.rs (+2) · كود: core/src/db/instances.rs، core/src/db/schema.rs (+4) |
### profiles

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-102** | إنشاء بروفايل مخصّص | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/instances.rs، core/src/instances/metadata.rs (+1) · كود: core/src/instances/registry.rs، core/src/journal/mod.rs |
| **F-103** | ربط مجلد موجود | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/instances.rs، core/src/instances/registry.rs · كود: core/src/db/instances.rs، core/src/db/schema.rs (+2) |
| **F-104** | **عمود `source`** على النسخ | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/instances.rs، core/src/db/mod.rs (+1) · كود: core/src/db/instances.rs، core/src/db/schema.rs (+3) |
| **F-105** | ~~`ownership_policy` مُنفَّذ~~ | Drop | — | مُستبعد (قرارُ المواصفة) | — |
| **F-106** | معاينة إنشاء على مجلد موجود | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/registry.rs · كود: core/src/instances/registry.rs، src/app/pages/NewInstanceDialog.tsx |
| **F-107** | **تراجع إنشاء كامل** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/registry.rs · كود: core/src/instances/registry.rs |
| **F-108** | تفعيل/تعطيل (إعادة تسمية) | Keep | M2 | حاضر — مُراجَع | core/src/instances/toggle/mod.rs::preview_toggle · core/src/instances/toggle/mod.rs::preview_one (top-level, already-disabled, `.jar`-only, symlink, outside-root and destination-collision guards with the legacy sentences) · core/src/instances/toggle/mod.rs::execute_toggle · src-tauri/src/commands.rs::preview_toggle / ::execute_toggle · src/app/pages/InstanceDetails.tsx::InstanceDetails (preview list showing `allowed: false` + reason, then confirm) · src/lib/ipc.ts::previewToggle / ::executeToggle |
| **F-109** | **تراجع تنفيذي عكسي** | Keep | M2 | حاضر — مُراجَع | core/src/instances/toggle/mod.rs::execute_toggle (first failure marks every remaining allowed entry `Skipped`, then walks `completed` in reverse) · core/src/instances/toggle/mod.rs::ToggleState (Success|Failed|Skipped|RolledBack|RollbackFailed|RollbackSkipped) |
| **F-110** | رفض الروابط الرمزية والتجاوز والتصادم | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/toggle/tests.rs |
| **F-111** | حذف بمشغّل أمان **بلا حذف دائم** | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/remove/tests.rs، src-tauri/src/commands/tests.rs · كود: cli/src/commands/remove.rs، core/src/instances/remove/mod.rs (+4) |
| **F-112** | كلمة تأكيد `REMOVE` للملفات الخارجية | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/remove/tests.rs · كود: core/src/instances/remove/mod.rs، src-tauri/src/commands.rs (+2) |
| **F-113** | نسخ احتياطي كامل للبروفايل | Keep | M2 | حاضر — مُراجَع | core/src/instances/backup/mod.rs::create_backup · core/src/instances/backup/manifest.rs::BackupManifest · src-tauri/src/commands/backup.rs::backup_instance · src/app/pages/InstanceBackup.tsx |
| **F-114** | استعادة بحفظ الوجهة القائمة جانباً | Keep | M2 | حاضر — مُراجَع | core/src/instances/backup/restore.rs::build_restore_plan · core/src/instances/backup/restore.rs::execute_restore · core/src/instances/backup/restore.rs::rollback · src-tauri/src/commands/backup.rs::restore_instance |
| **F-115** | **نسخ `saves` اختياري** | Keep | M2 | حاضر — مُراجَع | core/src/instances/backup/restore.rs::recheck_entry · core/src/instances/backup/mod.rs::create_backup · src-tauri/src/commands/backup.rs::restore_instance |
| **F-116** | مدير نسخ التحديث (لكل ملف) | Defer | later | مؤجَّل (قرارُ المواصفة) | — |
| **F-117** | إصلاح الميتافيزقا (خطة) | Defer | later | مؤجَّل (قرارُ المواصفة) | — |
| **F-118** | معاينة العوالم (قراءة فقط) | Keep | M3 | حاضر — مُراجَع | core/src/instances/backup/worlds.rs::preview_worlds · src-tauri/src/commands/backup.rs::preview_instance_worlds |
| **F-119** | استيراد عالم (نسخ فقط) | Defer | later | مؤجَّل (قرارُ المواصفة) | — |
| **F-181** | استيراد بيانات التطبيق القديم (باستود) | Keep | later | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/legacy/import_tests.rs، core/src/legacy/tests.rs (+1) · كود: core/src/legacy/import.rs، core/src/legacy/mod.rs (+6) · اختبار (ذكر): core/src/legacy/import_tests.rs، src-tauri/src/commands/legacy_tests.rs · وثائق: docs/adr/0003-scope-decisions.md، docs/adr/0006-legacy-import.md (+1) |
### updates

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-120** | معاينة التحديث | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/updates/tests.rs، src-tauri/src/commands/updates_tests.rs · كود: core/src/updates/mod.rs، src-tauri/src/commands.rs (+1) |
| **F-121** | **إعادة تحقق قبل التنفيذ** | Keep | M2 | جزئي — مُراجَع | E:\ModManager\cli\src\commands\update.rs::update (lines 39-45: single read_lock; lines 111-186: confirm then swap, no re-verify) · E:\ModManager\src-tauri\src\commands.rs::check_instance_updates (read-only F-120 only) · E:\ModManager\src-tauri\src\lib.rs::generate_handler (no update-install command) |
| **F-122** | عقد السلامة (5 بنود) | Keep | M2 | مفقود — مُراجَع | E:\ModManager\cli\src\commands\mod.rs::confirm_forced (line 174) · E:\ModManager\cli\src\commands\update.rs::update (line 111) |
| **F-123** | **نسخ احتياطي Yes/No/Cancel** | Keep | M2 | مفقود — مُراجَع | E:\ModManager\cli\src\commands\update.rs::update (lines 138-147: unconditional rename to backup) · E:\ModManager\core\src\instances (no profile backup module) |
| **F-124** | تحديث بالتنزيل فقط | Keep | M2 | مفقود — مُراجَع | E:\ModManager\cli\src\commands\update.rs::update (lines 113-158: direct-to-mods download and swap) |
| **F-125** | مخطّط الإصدار المستهدف | Keep | M3 | مفقود — مُراجَع | E:\ModManager\core\src\updates\mod.rs::check_updates (module header: 'Installing a newer version is a separate flow ... deliberately not part of this module') · E:\ModManager\src-tauri\src\commands.rs::check_instance_updates |
| **F-126** | اقتراح هدف مشترك | Keep | M3 | مفقود — مُراجَع | workspace grep for 'target_version|common_target|suggest' — only export/instance settings fields |
| **F-127** | **تنظيف الهدف يترك المجهول** | Keep | M3 | مفقود — مُراجَع | workspace grep for 'cleanup' — only pack staging and db cache cleanups |
| **F-128** | `installed_mods.json` مانيفست | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/record.rs، core/src/instances/tests.rs · كود: core/src/instances/tests.rs، core/src/updates/mod.rs |
| **F-129** | مسح المودات المحلية + استيراد | Keep | M3 | حاضر — بتتبّعِ اختبار | كود: core/src/updates/mod.rs |
| **F-130** | **قصّ الميتافيزقا فقط** | Keep | M3 | مفقود — مُراجَع | E:\ModManager\core\src\instances\registry.rs (line 151: 'reconcile' only in a comment) · E:\ModManager\core\src\instances\remove\mod.rs::execute_remove (the only lock-entry dropper) |
| **F-131** | **التمييز «موجود» عن «مؤهَّل»** | Keep | M3 | جزئي — مُراجَع | E:\ModManager\core\src\instances\mod.rs::LocalContentFact · E:\ModManager\core\src\instances\lock.rs::read_lock · E:\ModManager\src-tauri\src\dto.rs::LocalFactDto (project_id: Option<String>) |
| **F-132** | كاش لكل تشغيل في المخطّط | Keep | M3 | جزئي — مُراجَع | E:\ModManager\core\src\resolver\batch.rs::check (lines 37-52: RunCache::new(true) passed to resolve_dependency for the batch plan) · E:\ModManager\core\src\resolver\cache.rs::RunCache (12-field ResolveKey + HashKey, enabled bypass) · E:\ModManager\core\src\resolver\resolve\tests.rs::spec_f058_cache_reuses_duplicate_resolve |
### storage

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-133** | ترحيلات SQLite المرقّمة | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/mod.rs · كود: core/src/db/schema.rs |
| **F-134** | **بوابة إصدار المخطط المستقبلي** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/mod.rs، src-tauri/src/commands/tests.rs |
| **F-135** | نسخة احتياطية قبل الترحيل | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/mod.rs |
| **F-136** | **نشر النسخة بلا استبدال** | Keep | M1 | حاضر — بتتبّعِ اختبار | كود: core/src/db/mod.rs |
| **F-137** | **fail-closed عند القفل** | Keep | M1 | حاضر — مُراجَع | core/src/db/mod.rs::DbError::Busy · core/src/db/mod.rs::is_busy · src-tauri/src/commands.rs::db_error · src-tauri/src/dto.rs::ErrorCode::DatabaseBusy |
| **F-138** | **تنقية رسائل الترحيل** | Keep | M1 | حاضر — مُراجَع | core/src/db/mod.rs::DbError::MigrationFailed · core/src/db/mod.rs::file_name · src-tauri/src/state.rs::StartupFailure · src-tauri/src/lib.rs::report_startup_failure |
| **F-139** | **فشل قبل بناء النافذة** | Keep | M1 | حاضر — مُراجَع | src-tauri/src/state.rs::AppState::open_at · src-tauri/src/lib.rs::run · src-tauri/src/lib.rs::StartupFailure::window_failed · src-tauri/src/state.rs::STARTUP_FAILURE_EXIT_CODE |
| **F-140** | **WAL** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/mod.rs |
### operations

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-141** | **سجلّ عمليات ذرّي (JSON)** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/recovery.rs، core/src/journal/store/tests.rs (+2) · كود: core/src/downloader/batch.rs، core/src/downloader/mod.rs (+1) |
| **F-142** | انتقالات مُقيّدة للعمليات والعناصر | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/journal/store/tests.rs، core/src/journal/tests.rs · كود: core/src/journal/mod.rs |
| **F-143** | **كشف مجلد عمليات غير منتهية** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/journal/store/tests.rs، core/src/journal/tests.rs · كود: core/src/journal/mod.rs |
| **F-144** | **القفل عبر العمليات** | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: src-tauri/src/commands/pack_tests.rs |
| **F-145** | مركز العمليات (قراءة فقط) | Keep | M2 | جزئي — مُراجَع | E:\ModManager\src-tauri\src\commands.rs::list_pending_creates · E:\ModManager\src-tauri\src\commands\pack.rs::pending_pack_recoveries · E:\ModManager\src-tauri\src\lib.rs::generate_handler (no list-all-operations command) |
| **F-146** | **تقييم الأدلة — أعداد فقط** | Keep | M3 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/instances/recovery.rs، core/src/modpack/recovery_tests.rs (+1) · كود: core/src/instances/recovery.rs، src-tauri/src/commands/pack.rs (+5) · اختبار (ذكر): src-tauri/src/commands/pack_tests.rs |
| **F-147** | **تعليق كـ«مُهجر»** | Defer | later | مؤجَّل (قرارُ المواصفة) | — |
### logging

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-148** | تنقية الأسرار من السجلات | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/logging/sanitize.rs · كود: core/src/logging/sanitize.rs |
| **F-149** | **السجل المحلي كامل وغير منقّى** | Keep | M1 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/logging/mod.rs · كود: core/src/logging/mod.rs، core/src/logging/report.rs (+2) |
| **F-150** | **فشل مغلق بلا مخرج خام** | Keep | M1 | حاضر — مُراجَع | core/src/logging/sanitize.rs::Policy · core/src/logging/sanitize.rs::verify_redacted · core/src/logging/report.rs::write_report · core/src/logging/report.rs::discard · src-tauri/src/state.rs::dialog_text |
### settings

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-151** | إعدادات محفوظة (21 مفتاحاً) | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/mod.rs، src-tauri/src/commands/diagnostics_tests.rs · كود: src-tauri/src/commands.rs، src-tauri/src/dto.rs (+2) |
| **F-152** | سجلّ التنزيلات | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/download_history.rs، src-tauri/src/commands/channel_tests.rs (+1) · كود: core/src/db/download_history.rs، core/src/db/schema.rs (+4) · اختبار (ذكر): src-tauri/src/commands/channel_tests.rs، src-tauri/src/commands/history_tests.rs |
| **F-153** | **فهرس على سجلّ التنزيلات** | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/db/download_history.rs · كود: core/src/db/download_history.rs، core/src/db/schema.rs (+3) |
| **F-154** | المفضّلة | Defer | later | مؤجَّل (قرارُ المواصفة) | — |
### -

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-155** | تصدير بروفايل zip | Keep | M2 | حاضر — مُراجَع | src/app/pages/InstanceExport.tsx::InstanceExport (preview -> plan review -> Write, every edit invalidates the plan) · src/lib/ipc.ts::planModpackExport / writeModpackExport / listExportFolders · src-tauri/src/commands/pack.rs::plan_modpack_export, write_modpack_export, list_export_folders · core/src/modpack/export.rs::collect_overrides / exclusion_reason / index_json / write_export_zip · core/src/modpack/export.rs::EXCLUDED_DIR_NAMES (13) / EXCLUDED_FILE_NAMES (6) / EXCLUDED_SUFFIXES (13) / SENSITIVE_NAME_TOKENS (12) · src/i18n/en.json::export.planTitle|export.action.plan|export.action.write|export.count.files|export.count.excluded|export.skip.*|export.excluded.* |
| **F-156** | **قائمة الاستبعاد** | Keep | M2 | حاضر — بتتبّعِ اختبار | اختبار بمعرّف: core/src/modpack/export_tests.rs · كود: core/src/modpack/export.rs · اختبار (ذكر): core/src/modpack/export_tests.rs |
| **F-157** | `export_manifest.json` | Keep | M2 | مفقود — مُراجَع | E:\ModManager\core\src\modpack\export.rs::write_export_zip (lines 812-836: INDEX_ENTRY + overrides only) · E:\ModManager\core\src\modpack\export.rs::index_json (pack contents only, no exclusions) |
| **F-158** | **استيراد ميتا فقط بلا استخراج** | Keep | M2 | مفقود — مُراجَع | E:\ModManager\src-tauri\src\lib.rs::generate_handler (lines 50-52: legacy trio only) · E:\ModManager\core\src\legacy\import.rs::plan_import (old home only) |
| **F-159** | تصدير قائمة المودات (MD/CSV/TXT) | Drop | — | مُستبعد (قرارُ المواصفة) | — |
| **F-160** | استيراد/تصدير JSON | Drop | — | مُستبعد (قرارُ المواصفة) | — |
| **F-161** | اختيار هدف عالم لقاعدة البيانات | Defer | later | مؤجَّل (قرارُ المواصفة) | — |
### ui

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-162** | شريط تنقّل + صفحات مكدّسة | Keep | M2 | حاضر — مُراجَع | src/app/Shell.tsx::Shell (PAGE_IDS = instances|browse|modpack|downloads|settings, NAV) · src/App.tsx::App (page switch through document.startViewTransition) · src/app/pages/PlaceholderPage.tsx::PlaceholderPage · src/app/CommandPalette.tsx::CommandPalette · src/i18n/en.json::nav.instances|nav.browse|nav.modpack|nav.downloads|nav.settings |
| **F-163** | لوحة **Home** | Drop | — | مُستبعد (قرارُ المواصفة) | — |
| **F-164** | صفحة Discover | Keep | M2 | حاضر — مُراجَع | src/app/browse/Inspector.tsx · src-tauri/src/commands.rs::list_project_versions · src/lib/ipc.ts::listProjectVersions |
| **F-165** | جدول نتائج (13 عموداً) | Keep | M2 | حاضر — مُراجَع | src/app/browse/ResultsTable.tsx · src/app/browse/columns.ts · src/app/browse/useVirtualRows.ts · core/src/providers/mod.rs::Project |
| **F-166** | ترقيم «Load more» | Keep | M2 | حاضر — مُراجَع | src/app/browse/paging.ts · src/app/pages/BrowsePage.tsx · src/app/browse/selectionStore.ts |
| **F-167** | **سجلّ تنقّل 32 مدخلاً** | Drop | — | مُستبعد (قرارُ المواصفة) | — |
| **F-168** | **مراقب نظام الملفات (تلميح)** | Keep | M2 | حاضر — مُراجَع | core/src/watcher.rs · src-tauri/src/commands/watch.rs · src/app/lib/staleness.ts · src/app/pages/InstanceDetails.tsx |
| **F-169** | لوحة تنقّل لوحة المفاتيح (cmdk) | Keep | M2 | حاضر — مُراجَع | src/app/CommandPalette.tsx::CommandPalette (cmdk Command/CommandInput/CommandList/CommandGroup/CommandItem) · src/components/ui/command.tsx (shadcn command primitives wrapping cmdk) · src/app/Shell.tsx::Shell (Ctrl+K / Cmd+K toggles the palette, header button opens it) · src/components/Kbd.tsx::Kbd · src/i18n/en.json::palette.title|palette.description|palette.placeholder|palette.empty|palette.group.goTo|palette.hint.navigate|palette.hint.run|palette.escape|palette.shortcut |
| **F-170** | الثيم واللون | Keep | M2 | حاضر — مُراجَع | src/app/preferences.ts::THEMES (system|dark|light), applyToDocument, isDark, watchSystemTheme, usePreferences · src/main.tsx (applyToDocument before first paint, no flash of the wrong theme) · src/app/pages/SettingsPage.tsx::SettingsPage (settings.theme Segmented with theme.dark|theme.light|theme.system) · src/styles/theme.css (html.dark, color-scheme, tokens) · src/i18n/en.json::settings.theme|theme.system|theme.dark|theme.light |
| **F-171** | **اختصارات لوحة المفاتيح** | Keep | M2 | حاضر — بتتبّعِ اختبار | كود: src/app/CommandPalette.tsx |
| **F-172** | شريط عنوان مخصّص (إطار مخفي) | Drop | — | مُستبعد (قرارُ المواصفة) | — |
| **F-173** | RTL + i18n | Keep | M2 | حاضر — مُراجَع | src/app/i18n.ts (i18next + react-i18next, en/ar resources, Arabic plural categories, format hook for numerals) · src/app/preferences.ts::applyToDocument (root.lang, root.dir from Locale, DIRECTION map) · src/i18n/en.json + src/i18n/ar.json (all UI strings as keys; no hard-coded copy in components) · src/styles/theme.css (html[dir="rtl"] leading, logical direction rules) · src/app/Shell.tsx (ps-/pe-/ms-/me- logical spacing, RTL-mirrored rail glyphs) · src/lib/format.ts (formatNumber/formatBytes with locale + numeral system) · scripts/check-i18n.mjs (key parity + Arabic plural-category check, wired as pnpm i18n:check in .github/workflows/ci.yml) |
| **F-174** | تخطيط متجاوب | Keep | M2 | حاضر — مُراجَع | src/app/Shell.tsx::Shell (grid-cols-[auto_1fr], width 56/208 rail, min-w-0 + overflow containment) · src/app/pages/BrowsePage.tsx (sm:grid-cols-2 lg:grid-cols-3) · src/app/pages/InstancesPage.tsx (grid-cols-1 sm:grid-cols-2 lg:grid-cols-3, expanded card spans sm:col-span-2 lg:col-span-3) · src/app/pages/InstanceExport.tsx (grid-cols-1 sm:grid-cols-2 fields, flex-wrap rows) · src-tauri/tauri.conf.json (window 1200x800, minWidth 900, minHeight 600) |
### -

| المعرّف | السلوك | القرار | المرحلة | الحالة | الدليل |
|---|---|---|---|---|---|
| **F-175** | **الترخيص والتفعيل دون اتصال** | Drop | **قرارك: Drop نهائي.** تطبيق مجاني — لا نظام ترخيص ولا بصمة جهاز ولا نافذة تفعيل | مُستبعد (قرارُ المواصفة) | — |
| **F-176** | **بناء PyInstaller + Inno + AppImage** | Drop | Tauri bundler يحلّها (`foundations.md` §7) | مُستبعد (قرارُ المواصفة) | — |
| **F-177** | **نماذج العامة/الخاصة** | Drop | متعلّق بـF-175 | مُستبعد (قرارُ المواصفة) | — |
| **F-178** | **تشخيص ذاتي + قوالب بناء** | Drop | يستحق فقط مع الترخيص | مُستبعد (قرارُ المواصفة) | — |
| **F-179** | **وضع محمول** | Drop | `directories` يكفي | مُستبعد (قرارُ المواصفة) | — |
| **F-180** | **CI لاختبارات Python** | Drop | لا CI أصلاً؛ يكتب سطر واحد لـCargo | مُستبعد (قرارُ المواصفة) | — |

---

**ملاحظةٌ عن الأسلوب:** حيث وُجد السلوكُ في نسخة الرست فهو مُعادٌ بأسلوبِ رست: أخطاءٌ مُصنَّفةٌ عبر IPC
(`IpcError { code, message }`) بدلَ النصوصِ الحرّة، وخطةٌ قبل التنفيذ، وكتابةٌ ذرّيةٌ إلى `.part` مع فحصِ هاش
قبل إعادة التسمية، ورشّحاتُ مسارٍ قبل الاستخراج، وقنواتُ تقدّمٍ بدلُ حجبِ الواجهة. أمّا ما هو مفقودٌ فيبقى مفقودًا.
