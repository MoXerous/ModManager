# behaviors/downloads.md — التحميل، والتحقق من الهاش، وفحص المساحة، وإعادة التوجيه

> **المرجع:** `app\services\download_manager.py` (401 سطراً) · `app\services\artifact_verifier.py` (135) · `tests\qa\test_verified_artifact_pipeline.py` (660)
> لا يوجد CurseForge؛ انظر `providers.md` §0.

---

## 1. آلة الحالات

### 1-1. الأطوار — عبر `state_callback: Callable[[str, Path], None]`

| # | الطور | المسار المُبلَّغ |
|---|---|---|
| 1 | `"downloading"` | `part_path` |
| 2 | `"verifying"` | `part_path` |
| 3 | `"committing"` | **الوجهة النهائية** |

**أي طور آخر مرفوض** من المستهلك: `app/services/operation_tracking.py:152` يرفع `OperationTrackingError(f"unsupported download operation phase: {phase!r}.")`.

> **هذا تطبيق صارم لـ`AGENTS.md` §5.7 (تقدّم محدود البوتيرة)** — في Rust: `enum DownloadPhase { Downloading, Verifying, Committing }` غير قابل للبناء من نصّ.

### 1-2. الحالات النهائية — `DownloadResult.status`

| القيمة | المعنى | `.part` |
|---|---|---|
| `"success"` | نُزّل + تحقّق + التُزم | حُذف |
| `"skipped"` | الوجهة موجودة و`verify_artifact` نجح على بايتاتها القائمة — **بلا نقل وبلا أطوار** | لا يمسّه |
| `"cancelled"` | أوقفه المستخدم | **محفوظ عمداً للاستئناف** |
| `"failed"` | فشل التحقق (**`.part` يُحذف**) أو أي استثناء آخر (**`.part` يُحفظ**) | حسب الحالة |

قيم أخرى تُنسخ حرفياً من `resolved.compatibility_status`: `"compatible"` · `"incompatible"` · `"browse-only"` · `"dependency-error"` · `"resolve-error"` · `"selected-version"`.
و`"blocked"` تُنتج في `batch_install_plan.py:93`.

**تصنيف السجلّ** (`operation_models.py:32-40` `OperationItemState`): `queued` · `downloading` · `verifying` · `committing` · `completed` · `skipped` · `failed` · `cancelled`.

> **`"not-completed"` ليس حالة قائمة** — يُعَبَّر عنه **بغياب** طور `"committing"`. إن أردنا تسمية صريحة في Rust، فليكن `Phase::NotCommitted` مستنتَجاً، لا قيمة نصّية.

---

## 2. اصطلاح `.part`

```
part_path = target.with_suffix(target.suffix + ".part")
```

`artifact.jar` → `artifact.jar.part` (السطر 312).

- `.part` سليم = **قابل للاستئناف**.
- `.part` يُحذف عند: فشل التحقق (385)، أو تجاوز الحجم `expected_size` (317).
- `.part` **يُحفظ** عند: الإلغاء، وأي استثناء غير تحقّق.

> `AGENTS.md` §5.5 يفرض نفس الترتيب: `.part` → تحقّق هاش → **إعادة تسمية ذرّية**.

---

## 3. سياسة HTTPS

`_validate_url` (252-262) → `artifact_verifier.validate_download_url` إلا إذا مرّر المستدعي `url_validator`.

**يُستدعى في ثلاثة مواضع:**
1. على `task.file.url` **قبل** `mkdir` (285)
2. على كل `next_url` محسوب **قبل** الطلب (241)
3. على **`response.url`** عندما يعيد الخادم كتابة العنوان (225)

**السياسة في `validate_download_url`:**

| القاعدة | التفصيل |
|---|---|
| المضيف مطلوب | `urlsplit(url).hostname` لا `None` وإلا `ValueError("Download URL must include a host.")` |
| **لا بيانات اعتماد في الرابط** | `ValueError("Download URLs containing credentials are not allowed.")` |
| `https` إجباري | `ValueError("Download URL must use HTTPS.")` |
| `http` مسموح **فقط** لـ `{"127.0.0.1","::1","localhost"}` **وعملاً فقط** مع `allow_insecure_loopback=True` | الافتراضي `False`؛ الاختبارات تمرّر `True` |

> **نقطة عملية:** `allow_insecure_loopback` ضرورية للاختبارات التي تستخدم `ThreadingHTTPServer` حقيقي. في Rust: اجعلها إعداداً صريحاً يُفعَّل **فقط** في `#[cfg(test)]`.

---

## 4. فحص المساحةprevious الفحص

`_ensure_disk_space(destination_dir, expected_size, partial_size)` (68-78):

```python
remaining = max(0, expected_size - partial_size)
if remaining <= 0: return                      # لا فحص إطلاقاً عندما expected_size == 0
free = int(shutil.disk_usage(destination_dir).free)
if free < remaining:
    raise OSError(f"Insufficient disk space. Need {remaining} more byte(s), but only {free} byte(s) are free.")
```

| الملاحظة | التفصيل |
|---|---|
| **العتبة دقيقة تماماً** | لا هامش، ولا نسبة مئوية |
| لا يُحسب إلا **البايتات المتبقية** | لا يحسب تعايش `.part` مع الملف النهائي |
| **لا فحص إذا كان الحجم مجهولاً** | `expected_size == 0` ⇒ `remaining <= 0` ⇒ خروج فوري |
| **قبل أي طلب شبكة** | السطر 319 |

> **ضعف موثّق:** المودات تأتي من Modrinth/CDN فحجمها معروف عادةً، لكن الحزم الكبيرة قد تفشل بلا إنذار. **في Rust: اجعل `unknown size` حالة صريحة لا صمتة.**

---

## 5. الاستئناف عبر `Range`

`_download_once` (146-204):

```python
resume_from = part_path.stat().st_size if part_path.exists() else 0
if resume_from > 0: headers = {"Range": f"bytes={resume_from}-"}
```

| الحالة | السلوك |
|---|---|
| `resume_from > 0` **و** الردّ `206` | يُلحق |
| `resume_from > 0` **و** الردّ **`!= 206`** | `part_path.unlink(missing_ok=True)`، إغلاق، **إعادة طلب بلا ترويسات** ⇒ **إعادة بداية كاملة، لا إلحاق تالف** |
| `partial_size > expected_size > 0` | حذف `.part` (316-318) |
| `partial_size == expected_size > 0` | **تخطّي الشبكة كلياً**، ينتقل مباشرة إلى `verifying` (322) |

> **المبدأ:** 200-بدل-من-206 = بداية من جديد. **لا نثق أبداً في إلحاق لم يطلبه الخادم.**

---

## 6. إعادة المحاولة

### 6-1. ما يُعاد

`_raise_retryable_status` (137-144) يرفع `requests.HTTPError` لـ `429` أو `500 ≤ status ≤ 599`.
حلقة `for attempt in range(1, self.max_attempts + 1)` — `max_attempts=1` افتراضياً.

| قابل لإعادة المحاولة | غير قابل — يُرفع فوراً |
|---|---|
| `status_code is None` (نقل) | `4xx` آخر (مثل 404) |
| `429` | `3xx` مُتابَع |
| `500..599` | |

### 6-2. دالة التأخير

`retry_delay_seconds(attempt, headers, base_seconds, max_seconds)` (`artifact_verifier.py:59-80`):

1. `Retry-After` رقمي ← مقصوص إلى `[0, max_seconds]`
2. وإلا تاريخ HTTP عبر `parsedate_to_datetime` ← مقصوص إلى `[0, max_seconds]`
3. وإلا `base_seconds * 2 ** (attempt - 1)` ← مقصوص إلى `max_seconds`

**الافتراضات:** `backoff_base_seconds = 1.0` · `max_backoff_seconds = 30.0` (الأسطر 40-41). `sleep_func` قابلة للحقن (افتراضياً `time.sleep`) — تُستخدم للتأخير **و** لاستطلاع الإيقاف المؤقت.

### 6-3. الربط الإنتاجي

- `composition.py:70-72`: `DownloadManager(max_attempts=max_attempts)`
- `workers.py:1344, 2185, 2779`: `attempts = retry_failed_downloads + 1`، حيث `retry_failed_downloads` محصور `max(0, min(3, …))` والافتراضي `1`.
- **⇒ محاولتان في الإنتاج.**

> `foundations.md` §4.5 تطلب «إعادة محاولة بتأخير تصاعدي **مع عشوائية**». **`random` غير موجود في الكود القديم** — backoff حتمي. قرار مفتوح لك.

---

## 7. إعادة التوجيه

`_open_validated_response` (206-250):

- `allow_redirects=False` + **حلقة يدوية**
- `_REDIRECT_STATUS_CODES = {301, 302, 303, 307, 308}`
- `_MAX_REDIRECTS = 10`

| الشرط | النتيجة |
|---|---|
| `Location` مفقودة | `requests.TooManyRedirects("Redirect response did not provide a Location header.")` |
| `redirect_count >= 10` | `requests.TooManyRedirects("Download exceeded the 10-redirect limit.")` |
| `next_url = urljoin(response_url, location)` | **يُتحقق منه قبل الطلب التالي** |
| أي مسار خروج غير عائد | الردّ **يُغلق** (242-245) |

> السطر 248 فيه `raise` **ميت** (لا مسار يصله).

---

## 8. حارس سباق الروابط الرمزية

`_stream_response` (80-135) — **أهم حارس أمني في الملف**:

```python
path_guard(part_path)                          # 99  — قبل الفتح
with part_path.open("a+b") as artifact:        #     — وضع الإلحاق، بلا اقتطاع
    path_guard(part_path)                      # 102 — بعد الفتح، قبل أي كتابة
```

التعليق في المصدر: «Open without truncating, then revalidate the path before mutating the file. This closes the check/open window when a guarded staging parent is replaced.»

- الاستئناف: `artifact.seek(0, os.SEEK_END)` · الجديد: `seek(0)` ثم `truncate(0)`.
- `path_guard` يُنادى **~10 مرات أخرى** في `download_task` (المجلد، الوجهة، `.part`، الوجهة الفريدة، قبل التحقق، قبل الالتزام — الأسطر 288-374).
- `mrpack_installer.py:790-791` يزوّد `_validate_staging_download_path` بدل `path_guard`.

> **الدلالة على Rust:** النمط الصحيح هو **فتح بمقبض، ثم إعادة تحقق، ثم كتابة** — لا تحقق ثم فتح ثم كتابة. `AGENTS.md` §5.5 لا يذكر هذا صراحةً؛ **يستحق بنداً**.

---

## 9. التحقق — `ArtifactVerifier.verify_artifact(path, spec)`

**بترتيب صارم.** الفشل = `ArtifactVerificationError(ValueError)`.

| # | الفحص | الشرط | رسالة الفشل |
|---|---|---|---|
| 1 | ملف عادي **غير رابط رمزي** | `path.is_file() and not path.is_symlink()` | `"Downloaded artifact is not a regular file."` |
| 2 | **الحجم** | فقط إذا `expected_size > 0` | `"Artifact size mismatch. Expected {n} bytes, got {m}."` |
| 3 | **رفض HTML** — مُشغِّلان مستقلّان | (أ) `content_type` ∈ `{application/xhtml+xml, text/html}` (بعد `partition(";")`)، أو (ب) أول **1024 بايت** بعد `lstrip(b"\xef\xbb\xbf\x00\t\r\n ").lower()` تبدأ بـ `<!doctype html` أو `<html` أو `<head` أو `<body` | `"Downloaded artifact is an HTML response, not a content file."` |
| 4 | **SHA-1** | **يُحسب دائماً** (قطع 1 MiB)، يُقارن فقط إذا `expected_sha1` غير فارغ، غير حسّاس لحالة الأحرف | `"SHA1 mismatch. Expected {x}, got {y}."` |
| 5 | **SHA-512** | نفس المنطق | `"SHA512 mismatch. Expected {x}, got {y}."` |
| 6 | **سحر ZIP/JAR** | فقط إذا `Path(spec.filename).suffix.lower()` ∈ `{.jar, .mrpack, .zip}`؛ يشترط `zipfile.is_zipfile(path)` | `"Downloaded artifact is not a valid ZIP/JAR archive."` |
| 7 | **سلامة الأرشيف / سياسة القنبلة** | `inspect_zip_archive` بـ`DEFAULT_ARCHIVE_LIMITS` (انظر `archive-safety.md`)، ثم رفض إن **لم يوجد عنصر غير-مجلد** | نص `SafeArchiveError` حرفياً، أو `"Downloaded ZIP/JAR archive is empty."` |

### 9-1. ملاحظات وفجوات

| الملاحظة | الأثر |
|---|---|
| **لا فحص سحر JAR مستقل** (0x50 0x4B) | فحص ZIP ضمني عبر `is_zipfile` و**مدفوع باللاحقة لا بنوع المحتوى** |
| **لا فكّ ضغط إطلاقاً** | لا `testzip()`، لا `extract`، ولا قراءة بيانات أي عنصر. سياسة القنبلة **ميتافيزقا بالكامل** من `infolist()` |
| **الهاشان يُحسبان دائماً** حتى لو طُلب واحد — تمرّتان | في Rust: نُبقي الاثنين في تمريرة واحدة كما يفعل الكود |
| `ArtifactVerificationSpec` (frozen): `filename` · `expected_size=0` · `expected_sha1=""` · `expected_sha512=""` · `content_type=""` | |
| `ArtifactVerificationResult(size, sha1, sha512)` | **يُعاد فقط عند النجاح الكامل** |

> **لا حالة «skip»** على مستوى المُتحقِّق: غياب التوقّع (`expected_sha1 == ""`) يعني **«لم يُفحص»** وهو **غير قابل للتمييز عن النجاح** في القيمة المُعادة. **هذه ثغرة تصميمية نUCEIVE بلا إصلاح في Rust.**

---

## 10. الاستبدال الذرّي والإلغاء

### 10-1. الاستبدال

```python
part_path.replace(target)      # السطر 375 = os.replace — ذرّي، نفس نظام الملفات، يكتب فوق بصمت
```

> **تمييز مهم:** `app/ui/workers.py:966` استخدم **«non-replace rename»** في مُثبِّت `.mrpack`، لكن **`DownloadManager` نفسه لا يستخدم عدم-الاستبدال.** أي تعديل إلى ملف مستخدم موجود **محجوب** بالقفزة الأولى (§10.2)، لا هنا.

### 10-2. الملف الموجود مسبقاً

| الحالة | السلوك |
|---|---|
| `target.exists()` **و** `expected_sha1 or expected_sha512` | `verify_artifact(target, spec)`؛ نجاح ⇒ `status="skipped"`، `"File already exists and trusted metadata matches."`، **صفر أطوار، صفر شبكة** |
| `ArtifactVerificationError` | يسقط إلى `unique_path(target)` ⇒ **`artifact-1.jar`** (`utils/files.py:21-32`) |

> **ملف مستخدم موجود لا يُكتب فوقه أبداً.** هذا يلتزم بـ`AGENTS.md` §5.5 «لا تثق بملف غير متحقق» ويضيف حمايةٍ لم يذكرها المخطط.

### 10-3. الإلغاء

| الموضع | السلوك |
|---|---|
| قبل البدء | `DownloadResult(status="cancelled", message="Cancelled before download started.")` مع `path=None` (275-281) |
| أثناء البثّ | فحصان: داخل حلقة الإيقاف المؤقت (112-113) **و** مرة لكل قطعة (121-122) ⇒ `status="cancelled"`, `"Cancelled by user. Partial file kept for resume."`, `path=str(part_path)` |

**الإيقاف المؤقت:** `while pause_callback and pause_callback():` ثم فحص الإلغاء ثم `sleep(0.15)`. **يُستطلع قبل كتابة كل قطعة** ⇒ **القطعة المُجلبة سابقاً تُهمَل.**

---

## 11. متفرقات

| البند | القيمة | الدليل |
|---|---|---|
| اسم الملف الآمن | `safe_filename(task.file.filename)` **قبل أي شيء**؛ يستبدل `<>:"/\|?*` و`\x00-\x1F` بـ`_`، يقصّ، ويسقط إلى `"downloaded-mod.jar"` | `utils/files.py:8-10`، `download_manager.py:283` |
| حجم القطعة | `1024 * 256` = **256 KiB** | 110 |
| الإجمالي للعرض | `task.file.size or (resume_from + content_length if content_length else 0)` | 94 |
| الاستبدال الذرّي | `part_path.replace(target)` | 375 |

---

## 12. تصنيف الأخطاء ومسارها

**`DownloadManager` لا يمرّر أي استثناء.** كل شيء يُسطَّح إلى:

```python
DownloadResult(status="failed", message=str(exc) [ + " (partial file kept if present)" ])
```

- الإلغاء **حالة** لا استثناء.
- `workers.py:1368, 2200, 2793` تضيف `f" (failed after {attempts} attempt(s))"`.
- **كل الرسائل نصوص إنجليزية حرفية داخل كود الخدمة. لا رموز أخطاء ولا مفاتيح i18n.**

| النوع | الرسالة |
|---|---|
| `ArtifactVerificationError` | رسائل §9 |
| `ValueError` من `validate_download_url` | رسائل §3 |
| `OSError` من `_ensure_disk_space` | `"Insufficient disk space. Need {n} more byte(s), but only {m} byte(s) are free."` |
| `requests.TooManyRedirects` | رسالتا §7 |
| `requests.HTTPError` | `"Retryable HTTP status {code}."` — **إشارة تدفّق بحتة** |

> **النقل إلى `IpcError`:** يحتاج **تصنيف رموز حقيقياً**. `AGENTS.md` §5.3 يفرض `{ code, message }` والواجهة تترجم `code`. السلسلة الحالية لا تحمل معلومات قابلة للترجمة.

---

## 13. الاختبارات — التغطية والاحتجاجات

### 13-1. `ArtifactVerifierTests` (7) — قرص حقيقي، بلا شبكة
`test_accepts_archive_with_matching_size_sha1_and_sha512` · `test_rejects_size_mismatch` · `test_rejects_sha512_mismatch` · `test_rejects_html_and_corrupt_archives` · `test_archive_sanity_does_not_decompress_untrusted_entries` · `test_https_policy_rejects_remote_http_and_allows_explicit_loopback` · `test_retry_delay_prefers_retry_after_then_uses_capped_backoff`

### 13-2. `DownloadManagerArtifactPipelineTests` (17)
> `setUpClass` يشغّل **`ThreadingHTTPServer` حقيقي** على `127.0.0.1:0` — مقابس و HTTP/1.1 حقيقيان. ثلاثة اختبارات فقط تُحاكي `session.get`.

| الاختبار | emphasis |
|---|---|
| `test_download_commits_only_verified_artifact` | لا `.part` متبقٍ |
| `test_download_resumes_partial_file_with_range_request` | الخادم يستقبل `bytes={split}-` |
| `test_download_restarts_when_server_ignores_range` | **طلبان بالضبط**، بلا إلحاق تالف |
| `test_existing_mismatched_file_is_never_overwritten` | `artifact-1.jar` |
| `test_phase_callback_precedes_download_verification_and_commit` | الأطوار بالضبط `["downloading","verifying","committing"]` |
| `test_downloading_callback_failure_blocks_network_and_final_file` | **fail-closed**: فشل الطور يمنع الشبكة والملف |
| `test_committing_callback_failure_keeps_verified_partial_and_no_final_file` | `.part` المحقّق يبقى |
| `test_verified_existing_file_skip_emits_no_transport_phases` | `phases == []` |
| `test_cancel_keeps_partial_file_for_resume` | `.part` موجود، لا ملف نهائي |
| `test_retries_429_retry_after_and_503_backoff` | تأخير `0.0` و`0.25` |
| `test_does_not_retry_non_retryable_http_404` | **طلب واحد** رغم `max_attempts=3` |
| `test_rejects_insecure_final_url_after_redirect` | رفض بعد إعادة التوجيه |
| `test_custom_url_validator_checks_initial_and_final_redirect_urls` | المستدعى مرتين بالضبط |
| `test_custom_url_validator_blocks_redirect_before_requesting_destination` | الوجهة المحظورة **لا تُطلب** |
| `test_path_guard_rechecks_after_partial_handle_open_before_writing` | **يحتاج صلاحيات رابط رمزي ⇒ `SkipTest` على ويندوز** |
| `test_verification_failure_never_commits_and_discards_invalid_partial` | 4 حالات فرعية |
| `test_disk_preflight_blocks_download_before_request` | `disk_usage` مُحاكى بـ`free=1` |

### 13-3. `DownloadWorkerRetryContractTests` (1) ⚠
`test_workers_delegate_configured_retries_to_download_manager` — **يقرأ نص المصدر** ويتأكد أن `create_download_manager(attempts)` وردت 3 مرات وأن `DownloadManager()` القديمة غائبة. **لا يختبر سلوكاً؛ هشّ أمام أي إعادة هيكلة.**

### 13-4. الإحالات (flags)

| الاختبار | المشكلة |
|---|---|
| `test_archive_sanity_does_not_decompress_untrusted_entries` | **بائت**: يحقن `ZipFile.testzip` ليُرمى، والكود الإنتاجي **لا يستدعيه أصلاً** |
| `test_workers_delegate_configured_retries_to_download_manager` | تأكيد نص مصدر، لا سلوك |
| `test_path_guard_rechecks_after_partial_handle_open_before_writing` | **يتخطّى** على ويندوز بلا صلاحيات |

---

## 14. غير مُتحقَّق

| البند |
|---|
| لم أُشغّل أي اختبار — كل ما سبق قراءة مصدر |
| هل عشوائية backoff مطلوبة فعلاً (مقابل `foundations.md` §4.5) — قرار مفتوح |
| سلوك حذف `.part` عند فشل **الالتزام** (لا التحقق) — غير مُغطّى باختبار |
| هل `free < remaining` كافٍ عملياً — يُظهر التحديث الفاشل أن فحص المساحة كافٍ، لكن هذا تطبيق قديم |
