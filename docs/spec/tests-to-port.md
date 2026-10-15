# tests-to-port.md — اختبارات تستحق النقل إلى Rust

> **المصدر:** 33 ملف اختبار · **850 اختباراً** · 1,124,573 بايت في `tests/qa/`
> خط الأساس: `python -m pytest tests -q -p no:randomly` ⇒ **850 passed / 0 failed / 0 skipped** في **39.01s** (`docs/baseline.md:86-87`)

---

## 1. الإحصاءات

| البند | القيمة |
|---|---|
| ملفات الاختبار | **33** |
| الاختبارات | **850** |
| بايتات `tests/qa/` | **1,124,573** |
| مُجمَّعة فعلياً | `.pytest_cache/v/cache/nodeids` — 850 مُعرّفاً |
| **سابقة فشل مُسجَّلة** | `lastfailed = {}` — **لا إخفاقات مُسجَّلة** |
| تراكيب `xfail` | **صفر** |
| علامات `pytest.mark.*` | **صفر** — pytest مُشغِّل فقط، وكل اختبار `unittest.TestCase` |

### 1-1. أكبر عشرة ملفات

| الملف | سطراً | اختباراً |
|---|---|---|
| `test_import_export_resolver_contracts.py` | 5,475 | **166** |
| `test_navigation_history.py` | 1,978 | 54 |
| `test_profile_pages.py` | 1,268 | 45 |
| `test_profiles_page.py` | 823 | 35 |
| `test_home_page.py` | 954 | 34 |
| `test_operation_journal.py` | 1,216 | 34 |
| `test_mrpack_installer.py` | 1,489 | 31 |
| `test_operation_center.py` | 1,327 | 31 |
| `test_title_bar.py` | 340 | 30 |
| `test_profile_content_model.py` | 834 | 27 |

> ⚠️ `test_import_export_resolver_contracts.py` وحده = **20% من الاختبارات كلها**.

---

## 2. الاختبارات الثلاثة التي تفشل على exFAT (يجب مراجعتها)

| الاختبار | السبب |
|---|---|
| `test_posix_directory_sync_failure_retains_published_backup` | `assertRaises(OSError)` واسع يُبتلع فشل `os.link` على exFAT |
| `test_directory_sync_failure_rolls_back_and_retains_completed_backup` | نفس السبب |
| `test_path_identity_casefolds_dedupe_and_callback_without_changing_display_path` | الادّعاءات نجحت؛ **الفشل في `__exit__` أثناء تنظيف `TemporaryDirectory`** |

**تصنيف `baseline.md` §3:** ✅ **ليس علة تطبيق** — عيب تصميم اختبار.

**الإصلاح المقترح (نص `baseline.md` حرفياً):** «إمّا تُعلَّم بأنها تحتاج دعماً للروابط الصلبة، أو **تُضيَّق تأكيداتها لتحدّد النداء الفاشل تحديداً** لا أيّ `OSError`».

**في Rust:**

| الأصل | المقابل |
|---|---|
| `assertRaises(OSError)` حول `_publish_backup` | `assert_eq!(err.kind(), ErrorKind::AlreadyExists)` — **أدقّ وأقوى** |
| `assertRaises(OSError)` حول `_sync_directory` | حقن خطأ `io::Error` **بالنوع** المحدد |
| تنظيف `TemporaryDirectory` | **تختفي** — `tempfile::TempDir` في Rust يُزلِّل عند Drop، بلا `__exit__` |

> **⚠️ في Rust يُصبح الفارق بنيويّاً:** `std::fs::hard_link` على exFAT يُعيد `os error 1` أيضاً. **استعمل `E:` (NTFS) للاختبارات** — `AGENTS.md` §6 يشترطه أصلاً.

---

## 3. الترشيح حسب القيمة

### 3-A. **انقلها — قيمة عالية، تغطية سلوكية**

#### التحقق من الهاش والتنزيل (25 → `core/tests/download/`)
| اختبار بايثون | ماذا يُثبّت | اسم Rust مقترح |
|---|---|---|
| `test_download_commits_only_verified_artifact` | بايتات مطابقة، لا `.part` | `spec_f020_download_commits_only_verified_artifact` |
| `test_download_resumes_partial_file_with_range_request` | ترويسة `Range` تصل للخادم | `spec_f020_download_resumes_with_range` |
| `test_download_restarts_when_server_ignores_range` | **طلبان بالضبط، بلا إلحاق تالف** | `spec_f020_download_restarts_when_range_unsupported` |
| `test_existing_mismatched_file_is_never_overwritten` | `artifact-1.jar` | `spec_f020_never_overwrites_existing_user_file` |
| `test_verification_failure_never_commits_and_discards_invalid_partial` | حجم/SHA1/SHA512/HTML | `spec_f021_verification_failure_discards_partial` |
| `test_disk_preflight_blocks_download_before_request` | **صفر طلبات HTTP** | `spec_f020_disk_preflight_blocks_before_request` |
| `test_phase_callback_precedes_download_verification_and_commit` | ترتيب الأطوار | `spec_f020_phases_precede_their_work` |
| `test_does_not_retry_non_retryable_http_404` | **طلب واحد** رغم `max_attempts=3` | `spec_f022_does_not_retry_404` |
| `test_rejects_insecure_final_url_after_redirect` | رفض بعد إعادة توجيه | `spec_f019_rejects_insecure_redirect_target` |

> **على Rust: `wiremock` بدل `ThreadingHTTPServer`** (وهو في `AGENTS.md` §4). **⚠️ لكن** اختبار «الخادم يتجاهل `Range`» يحتاج خادماً يعيد 200 دائماً — `wiremock` يصلح.

#### المزوّد (16 → `core/tests/providers/`)
`test_long_server_wait_is_not_clamped_to_early_retry` · `test_rfc850_year_uses_rolling_fifty_year_window` · `test_malformed_retry_after_cannot_override_valid_long_reset` · `test_only_known_read_only_posts_retry` · `test_invalid_rate_headers_fail_as_rate_limit_without_sleep` · `test_permanent_errors_and_bad_json_are_private_and_closed` · `test_public_search_fallback_does_not_bypass_server_wait`

> **التقنية في Rust:** `reqwest` + `tokio::time::pause()` + `wiremock`. **بلا نوم حقيقي.**

#### أمان الأرشيف (12 → `core/tests/safety/`)
`test_rejects_unsafe_or_ambiguous_member_paths` (**7 أسماء**) · `test_rejects_casefold_duplicate_member_paths` · `test_rejects_unicode_normalization_collisions` · `test_rejects_encrypted_and_symlink_like_members` · `test_rejects_suspicious_compression_ratio_without_decompressing` · `test_rejects_member_count_individual_and_total_size_limits`

#### الترحيلات (36 → `core/tests/db/`)
**كلها قابلة للنقل почти حرفياً** — `rusqlite` + `rusqlite_migration` في `AGENTS.md` §4.
الأعلى قيمة:
`test_future_version_is_rejected_without_backup_or_write` (**بوابة `foundations.md` §14**) · `test_current_database_uses_fast_path_without_backup_or_validation` · `test_publish_backup_never_replaces_an_existing_destination` · `test_process_crash_after_ddl_rolls_back_andkeeps_backup` · `test_waiting_runner_rereads_version_and_migrates_only_once` · `test_commit_failure_rolls_back_schema_and_hides_raw_cause` · `test_invalid_registry_fails_before_creating_database_with_safe_context`

#### سجلّ العمليات (34 → `core/tests/journal/`)
`test_operation_transitions_accept_only_approved_edges_and_idempotency` (**مصفوفة 5×5**) · `test_item_transitions_accept_only_approved_edges_and_idempotency` (**8×8**) · `test_parser_rejects_schema_uuid_revision_duplicate_and_counter_tampering` (**6 عبثات**) · `test_revision_conflict_and_atomic_replace_failure_preserve_committed_bytes` · `test_concurrent_item_transitions_do_not_lose_updates` (24 خيطاً) · `test_discovery_is_read_only_and_separates_incomplete_terminal_and_issues` · `test_unfinished_item_causes_failed_operation_instead_of_false_completion`

#### المبدّل (15 → `core/tests/resolver/`)
`test_resolver_does_not_fall_back_from_fabric_to_forge_for_mod_files` (**حارس التعارض الكاذب**) · `test_resolver_prefers_preferred_version_id_before_latest` · `test_dependency_review_candidates_include_required_chain_for_optional_candidates` (**سلسلة 3 أعماق**) · `test_resolver_cache_reuses_duplicate_best_version_result`

#### مُثبِّت `.mrpack` (31 → `core/tests/modpack/`)
`test_target_race_is_not_overwritten_or_deleted` · `test_same_size_override_tampering_after_preview_is_rejected` (**تلاعب بنفس الطول!**) · `test_rollback_refuses_missing_or_mismatched_marker` · `test_journal_or_receipt_failure_never_publishes` · `test_every_precommit_persistence_failure_rolls_back_new_row_and_owned_root` (**6 subTests**) · `test_profile_row_rollback_failure_preserves_owned_root_for_recovery`

#### تنقية السجلات (18 → `core/tests/logging/`)
كلها قابلة للنقل **كت سلوك**. **اختباراتُ نص المصدر الثلاثة (§4) تُحذف وتُستبدل.**

#### مستودع المحتوى (21 → `core/tests/inventory/`)
`test_records_are_frozen_and_use_tuple_collections` · **`test_filename_and_project_id_alone_never_prove_ownership`** · **`test_managed_disabled_duplicate_incompatible_facts_coexist`** · `test_compatibility_requires_positive_metadata_evidence` (**جدول 7**) · `test_validation_errors_name_the_record_and_field_without_values`

---

### 3-B. **لا تنقلها — قيمة منخفضة أو بلا معنى في Rust**

| الاختبار | السبب |
|---|---|
| **كل `*StaticContractTests` (~150 اختباراً)** | **تأكيدات نص مصدر.** تنكسر على أي إعادة تسمية دون تغيير سلوكي. **الأسوأ: 30 اختباراً في `ActiveProfileRoutingStaticContractTests` تغطّي الواجهة كلها نصّاً.** |
| `test_windows_publish_uses_non_replace_rename` / `test_linux_publish_uses_renameat2_noreplace` | يحقنان `sys.platform` — **لا يختبران المنصّة**. في Rust، افحص `cfg(target_os)` مباشرة |
| `test_title_bar.py::TestTitleBar` (16 اختباراً) | تدّعي على **سمات خاصة** `_bg` · `_text_color` · `_accent_hex` وقيم hex حرفية. **واجهة Tauri أعصر — صفر منها ينتقل** |
| `test_window_native.py::WindowNativeTypedWin32Tests` (10) | `@skipUnless(sys.platform=="win32")` — **10 تخطّيات دائمة على لينكس**. Tauri يتولّى النافذة |
| كل اختبارات `test_navigation_history.py` (54) · `test_home_page.py` (34) · `test_profiles_page.py` (35) · `test_profile_pages.py` (45) · `test_profile_content_model.py` (27) | **مجموعها 195 اختباراً لواجهة PySide6.** Wouter + React **لا تحتاجها** |
| `test_operation_center.py::OperationCenterDialogTests` (13) + `…MainWindowTests` (6) | حوارات Qt |
| `UnifiedBuildCommandStaticContractTests` (7) | تأكيد على نصوص `build.ps1` |

---

## 4. ⚠️ علامات على اختبارات **واسعة جداً** أو **بائتة**

### 4-1. **بائتة — لا تختبر شيئاً**

| الاختبار | العلة |
|---|---|
| `test_profile_json_contract_contains_all_preferred_version_fields` (العقد:996) | **يبني قاموس بايثون، يمرّره `json.dumps`→`json.loads`، ويؤكد أن مفاتيح القاموس موجودة.** **لا يستدعي كود الإنتاج إطلاقاً.** قيمة التغطية صفر |
| `test_archive_sanity_does_not_decompress_untrusted_entries` | يحقن `ZipFile.testzip` ليُرمى — **والكود الإنتاجي لا يستدعيه أصلاً** |
| `test_content_inventory.py::test_records_are_frozen_and_use_tuple_collections` | ستّ تأكيدات `assertIsInstance(..., tuple)` متطابقة = ضجيج |

### 4-2. **واسعة — عدّة سلوكيات في دالة**

| الاختبار | سلوكيات |
|---|---|
| `test_operation_lease.py::…test_download_worker_releases_lease_after_failure_cancellation_and_journal_failure` | **3 سيناريوهات** في method واحد |
| `test_main_window_owns_one_journal_and_passes_it_to_approved_workers` | **~20 تأكيداً** عبر **3 وحدات** نصّية + عدّادات `count()` + ترتيب `source.index()` |
| `test_existing_legacy_database_gets_verified_pre_migration_backup` | ~25 تأكيداً على قاعدتين |
| `test_v002_is_additive_idempotent_and_backfills_only_explicit_evidence` | 9 تأكيدات: أعمدة + 7 قيم + جدول آخر + idempotency |
| `test_rejects_unsafe_or_ambiguous_member_paths` | 7 أسماء في assertion واحد |
| `test_wait_boundaries_and_obsolete_http_dates` | 4 صيغ تاريخ + حالتا حالة |
| `test_rejects_member_count_individual_and_total_size_limits` | 3 حدود |
| `test_validate_profile_classifies_unknown_duplicate_and_incompatible_local_mods` | 12 تأكيداً على 4 فئات |

### 4-3. **مقتصرة على تنفيذ بايثون**

| الاختبار | الكارثة |
|---|---|
| `source[start:start+700]` في `test_task_controller.py` و`test_content_provider.py` | **إزاحات أحرف سحرية** داخل `main_window.py` — **تؤكد على الدالة الخاطئة لحظة تحرّك الملف** |
| `source.count("_resolve_projects_threaded(") == 5` | أرقام سحرية مكرّرة في ملفَّي اختبار |
| `assertNotIn("worker", source.casefold())` | **يفشل على كلمة «worker» في أي تعليق** |
| `assertNotIn("os.", source)` | يطابق أي `os.` في النثر |

### 4-4. **خطر SRAM في Rust**

| البند | القرار |
|---|---|
| 195 اختبار واجهة PySide6 | **بلا قيمة في Tauri** |
| 150 اختبار نص مصدر | **حُذف** |
| `test_report_writer_has_no_raw_output_optout` (فحص توقيع) | بلا معنى |
| `test_canonical_inventory_has_no_io_framework_or_adapter_deps` (AST) | **بلا معنى في Rust — الأنواع والوحدات تفرض ذلك** |

---

## 5. الاعتماد على نظام الملفات

| الصنف | العدد | ملاحظة |
|---|---|---|
| **يحتاج NTFS / روابط صلبة** | **3** | `storage.md` §3.2 · `baseline.md` §3 |
| **يحتاج صلاحيات رابط رمزي** ⇒ `skipTest` | **12 موقعاً** | lease(2) · journal(2) · recovery_assessment(3) · mrpack(3) · artifact(1) · watcher(1) |
| **ⓘ تخطّي صامت** ⚠ | **1** | `test_world_import_blocks_existing_target_and_symlink_sources` — **`return` بدل `skipTest`** ⇒ يُبلَّغ PASS والنصف الثاني غير مُختبَر |
| `tempfile.TemporaryDirectory` | كل اختبار قرص | مع `ignore_cleanup_errors=True` في ملفَّي Qt |
| **`skipUnless(win32)`** | **10** | `test_window_native.py` |

> **كل الاختبارات تستخدم مجلدات مؤقتة — لا تلمس مجلدات mods حقيقية.** ✅ يلتزم `AGENTS.md` §6.

---

## 6. الشبكة والوقت

### 6-1. **المجموعة نظيفة خارج الشبكة** ✅
- **صفر طلب HTTP خارجي.**
- `test_verified_artifact_pipeline.py`: خادم **`ThreadingHTTPServer` حقيقي على `127.0.0.1:0`** (مقبس حقيقي، منفذ يختاره النظام)
- `test_modrinth_api_resilience.py`: `patch(session.request)` + `patch(time.sleep)` — صفر مقابس
- كل ما_else: بدائل مكتوبة يدوياً (`FakeModrinthClient` · `RecordingClient` · `_FakeDownloadManager` · `_FakeUser32`)
- **لا wiremock · لا responses · لا cassettes مسجَّلة**

### 6-2. التحكم في الوقت ✅
`test_modrinth_api_resilience.py` وحده يتحكّم بـ`time.time()` مثبّتاً — **حتمي**.
`ProfileManager(repo, clock=lambda: <fixed ISO>)` في `test_profile_aggregate.py:304` و`test_mrpack_installer.py:1082`.

### 6-3. ⚠️ أسطح التذبذب
| البند | العدد | الخطر |
|---|---|---|
| **`QTest.qWait(140-180)`** | **21** | `discover_incremental_loading`(13) · `profile_pages`(3) · `profiles_page`(1) · `navigation_history`(2) · `profile_content_model`(1) |
| **`QEventLoop` + `singleShot(3000)`** | 2 | حتى 3 ث لكل انتظار |
| **عمليات فرعية حقيقية** | ~16 استدعاء | lease (حتى 12 × `timeout=15`) · migrations (`timeout=10/15/30`) |
| **`time.sleep(0.015)` / `(0.1)`** | 2 | لفرض تشابك الخيوط |
| `Barrier.wait(timeout=0.25)` | 1 | مقصود |

> **على Rust: `tokio::time::pause()` + `advance()` تُلغي هذه الفئة كلها.** أقوى مكسب من النقل.

---

## 7. مُشغِّل الاختبارات

```bash
# القياسي (موثّق في 4 مواضع)
python -m unittest discover -s tests/qa -p "test_*.py" -v

# خط الأساس المقيس
python -m pytest tests -q -p no:randomly        # 850 passed / 0 failed / 0 skipped / 39.01s
```

| البند | القيمة |
|---|---|
| `conftest.py` · `pytest.ini` · `tox.ini` · `setup.cfg` | **صفر** |
| `pyproject.toml` | 356 بايت — `[project]` و`[tool.ruff]` فقط. **لا قسم pytest، لا تغطية، لا علامات** |
| **CI** | **غير موجود.** الملف الوحيد `.github/workflows/build-linux-appimage.yml` **لا يثبّت التبعيات ولا يشغّل الاختبارات إطلاقاً** |
| `ruff` | **غير مثبّت** — يُسجَّل «`No module named ruff`» في عشرات التقارير |

### 7-1. العزل
- **بلa `conftest.py` ولا صنف أساس.** العزل لكل اختبار.
- كل اختبار قرص ⇒ `tempfile.TemporaryDirectory()` خاصة به.
- كل اختبار قاعدة ⇒ مسار `db_path` صريح تحت جذر مؤقّت خاص.
- `MODRINTH_BULK_DOWNLOADER_HOME` ⇒ **اختبار واحد فقط** (فرعي `main.py --self-check`).
- 11 ملفاً ⇒ `os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")`
- `shutil.disk_usage` محقونة في **مكانين**

> **في Rust:** `tempfile::TempDir` لكل اختبار + **متغيّر بيئة واحد** `MODMANAGER_TEST_HOME` ⇒ كل شيء معزول بنيويّاً.

---

## 8. الميزانية المقترحة للنقل

| الفئة | العدد | الأولوية |
|---|---|---|
| الأمان (أرشيف + تحقق + هاش) | ~30 | **P0 — لا تفاوض** |
| التحقق من المُزود (retry/rate-limit) | 16 | **P0** |
| الترحيلات + البوابة | 36 | **P0** |
| سجلّ العمليات (بلا واجهة) | ~25 | **P1** |
| المبدّل + مخزون المحتوى | ~30 | **P1** |
| `.mrpack` (بلا واجهة) | ~20 | **P2** |
| البروفايل (تحقق/تفعيل/نسخ) | ~25 | **P2** |
| تنقية السجلات | 18 | **P1** |
| التنزيل (بلا خادم حقيقي — `wiremock`) | ~15 | **P1** |
| **واجهة** | **0** | **Tauri + React يُختبران بـ `pnpm build` + Playwright (مؤجَّل)** |
| **تغليف/ترخيص** | **0** | **Drop** |

**اقتراح: 215 اختباراً من 850 (25%)** — والباقي لا معنى له في Rust.

> **⚠️ الأرقام تقديرية مشتقّة من جداول الملفات.** لم أُشغّل أي اختبار. **غير مُتحقَّق.**

---

## 9. غير مُتحقَّق

| البند |
|---|---|
| **لم أُشغّل أي اختبار** — كل الأرقام من `.pytest_cache` وقراءة المصدر |
| أجسام الاختبارات الموسومة ⚠ في تقرير الوكيل (مُشتقّة من الاسم والفئة) |
| عدد الاختبارات الفعلي بعد Transfers قد يختلف (بعضها يندمج) |
| `test_navigation_history.py` · `test_home_page.py` · `test_profiles_page.py` · `test_profile_pages.py` · `test_operation_center.py` — أسماء فقط |
| ما إذا كانت **Playwright** مطلوبة لاختبارات الواجهة (لم يُحسم) |
