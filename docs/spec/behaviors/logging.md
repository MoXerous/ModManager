# behaviors/logging.md — تنقية الأسرار والقرارات من السجلات

> **المرجع:** `app\services\log_sanitizer.py` (502 سطراً) · `app\diagnostics.py` (553) · `tests\qa\test_log_sanitizer.py` (488، **18 اختباراً**)
> `foundations.md` §14 يطلب صراحةً «تنقية السجلات من الأسرار».

---

## 1. القاعدة المركزية: **فشل مغلق، بلا مخرج خام**

| البند | السلوك |
|---|---|
| `sanitize` يرفع استثناءً | **يُجهض كتابة التقرير**، ولا يُكتب ملف، ولا يُوجد مخرج خام |
| المخرج الخام اختياري؟ | **لا — `_write_report` لا يملك معامل `redact`** |
| **سجل النشاط المحلي** | **مقصود أن يبقى كاملاً وغير منقّى** |
| التقارير القابلة للمشاركة | تمرّ كلّها بالسياسة المركزية |

> **لماذا السجل المحلي غير منقّى؟** لأنه سجلّ تنقيح للمستخدم على جهازه؛ تنقيته تفقد قدرته على تشخيص مشكلته. **حدود واضحة، موثّقة، ومُختبَرة.**

---

## 2. ما الذي يُنقّى — من أسماء الاختبارات الثمانية عشر

| # | الفئة | التفاصيل |
|---|---|---|
| 1 | **الهوية والبيت والترخيص** | 13 حقلاً منقّى؛ 9 أسرار غائبة؛ **`license_state` محفوظ** |
| 2 | **المسارات المطلقة** | 4 سياقات ⇒ **اسم الملف (basename) فقط** |
| 3 | **النسخ والإصدارات والأسطر** | النص غير المتأثر يبقى، **idempotent**، واجهة «أسطر» مستقرة |
| 4 | **الترويسات والمفاتيح والمفاتيح الخاصة** | 5 أشكال ترويسة/مفتاح؛ **`token_count` و`password_check` تُلمَسان** |
| 5 | **روابط URL** | userinfo · `token` · `#access_token` منقّاة؛ **`page` و`sort` محفوظان** |
| 6 | **عناوين IP** | IPv4 وIPv6 مع منفذ منقّاة؛ **`1.21.1` والطوابع ISO محفوظان** |
| 7 | **JWT + بيانات اعتماد مضمّنة** | IPv6 مجرد + 5 أشكال اعتماد |
| 8 | **اعتماديات ذات بادئة مزوّد** | 5 أشكال |
| 9 | **JSON متشعّب** | تنقّى تكرارياً، الحقول الآمنة سليمة |
| 10 | **JSON مشوّه** | يتراجع بأمان + idempotent |
| 11 | **كتابة التقرير** | **بلا مخرج خام اختياري** (فحص التوقيع) |
| 12 | **الغلاف القديم** | `app.diagnostics` يعيد تصدير الثوابت المركزية |
| 13 | **الإخراج** | الطرفية + ملف التقرير **كلاهما** منقّى |
| 14 | **فشل المنقّي** | **يُجهض، بلا احتياطي خام** |
| 15 | **سجل النشاط** | يبقى كاملاً ولا يستدعي المنقّي (تأكيد نص مصدر) |
| 16 | **التقارير القابلة للمشاركة** | **لا تقرأ** السجل الخاص أو مخزن العمليات أو ملفات الأسرار |
| 17 | **تقرير تغليف الترخيص** | ينقّي الحمولة الكاملة + أسطر الطرفية |
| 18 | **اختبار سلوكي للتغليف** | **يحمّل سكربت التغليف بالمسار وينفّذ `main()`** بـargv/stdout مُحاكى |

### 2-1. المسارات — القاعدة (اختبار 2)

`test_redacts_absolute_path_contexts_and_preserves_basename` — 4 سياقات منقّاة إلى basename.
> **متوافق مع `storage.md` §3.5** (رسائل الترحيل تكشف اسم الملف لا المسار) و`journal.md` §1.7 (تقرير الإقلاع يكشف basename فقط).

### 2-2. السجل الخاص — القائمة السوداء (اختبار 16)

`sanitize` يقرأ من: `activity.log` · journal dir · ملفات الأسرار ⇒ **ممنوعة في دوال التقارير القابلة للمشاركة**.

---

## 3. الغلاف القديم — تبعية في الاتجاه الخاطئ عمداً

```python
# app/diagnostics.py
from app.services.log_sanitizer import (...)
```
`test_legacy_diagnostics_wrappers_delegate_to_central_policy` يؤكد أن `app.diagnostics` **يعيد تصدير الثوابت المركزية نفسها** — لا نسخة.

> **نقل إلى Rust:** `mod diagnostics { pub use crate::logging::sanitize::*; }` أو ببساطة **احذف الطبقة القديمة**. تنقيتان تساوي تنقيتان لا شيء.

---

## 4. ثلاث نقاط فاصلة — اختبارات تفقد قيمتها عند النقل

| الاختبار | الفرضية |
|---|---|
| `test_report_writer_has_no_raw_output_opt_out` | فحص **توقيع** `_write_report`: **لا معامل `redact`** |
| `test_activity_log_keeps_complete_local_message_and_does_not_call_sanitizer` | **نص مصدر** على `MainWindow.log` |
| `test_shareable_reports_do_not_read_private_log_journal_or_secret_files` | **قائمة استيرادات/استدعاءات ممنوعة** على 3 دوال تقارير |

> ⚠️ **ثلاثة من ثمانية عشر اختباراً نص-مصدر.** عند النقل إلى Rust **استبدلها بسلوك حقيقي**: اكتب ملف سجل فيه أسرار، اقرأ التقرير، assert الغياب. **هذا ممكن فعلاً في Rust ويسمح للتقرير النهائي بأن يكون ملفاً واحداً حقيقياً.**

---

## 5. أسرار اصطناعية — قائمة مُثبَتة

> **⚠️ ثغرة! (القيم أدناه اصطناعية بالكامل، ولا سرّ حقيقي في المشروع):** hunter2 · api-secret-value · secret-cookie · cli-secret · other-secret · jvm-secret · query-secret · fragment-secret · first/second-secret · provider-secret · access-secret · db-secret · cli-provider-secret · bearer-secret · basic-secret · json-secret · url-secret · malformed-secret · console-secret · report-secret · raw-secret · `MBD-ACTREQ-v1.<fake>` · CUSTOMER-123 · PRIVATE-LICENSE-ID · abcdef0123456789

**كلها قيم اختبارية مقصودة لإثبات التنقية.** **لا مفتاح CurseForge ولا رمز Modrinth ولا مفتاح ترخيص حقيقي في `tests/qa/` أو `packaging/`.**

**هويّات اصطناعية:** Alice · Alice Example · alice@example.com · `C:\Users\Alice` · /home/alice · LAPTOP-MUXEROUS

**مادة تشفيرية اصطناعية فقط:** `"a"*40` / `"b"*128` / `"a"*64` / هاشات ثابتة / UUIDs ثابتة / JWT ثابت.

> **⚠️ تنبيه تصفية:** عند نقل الاختبارات إلى Rust — **لا تنسخ هويّات حقيقية.** استخدم `alice` و`example.com`.

---

## 6. التقاط: سجلّ العمليات لا يسرّب أسراراً

`test_serialized_tracker_record_contains_no_transport_or_secret_fields` (اختبار 18):
الحقل **أسرار** لا يحتوي أياً من `http` · `header` · `token` · `license` · `file_content`.

وكذلك: `test_operation_record_roundtrip_is_lossless_and_counts_are_derived` يؤكّد أن الحمولة لا تحوي `url` ولا `token`.

> **في Rust:** هذا ينعكس على تصميم DTO — **الحقل `Item` في `operation_models` لا يملك حقل عنوان URL أصلاً.**

---

## 7. تقاطع مع بقية المواصفات

| المواصفة | التقاطع |
|---|---|
| `storage.md` §3.5 | `_safe_error_message`: اسم ملف لا مسار |
| `journal.md` §1.7 | تقرير الإقلاع: basename فقط |
| `providers.md` §2.10 | `ModrinthApiError` لا يحوي جسم الاستجابة (`"private"` غائب — اختبار) |
| `downloads.md` §12 | رسائل التحميل نصوص حرفية بلا رموز |

---

## 8. سلوكيات مرغوبة (استنتاج)

| # | السلوك | الدليل |
|---|---|---|
| 1 | **idempotent** | `test_preserves_status_versions_newlines_and_is_idempotent` · `test_malformed_structured_text_falls_back_safely_and_is_idempotent` |
| 2 | **fail-closed** | `test_sanitizer_failure_aborts_report_without_raw_fallback` |
| 3 | **الأرقام ليست عناوين** | `1.21.1` وISO-8601 محفوظان |
| 4 | **الحقولhops محفوظة** | `token_count` و`password_check` و`license_state` و`page` و`sort` |
| 5 | **القوائم متشعبة** | JSON متشعّب يُنقّى تكرارياً |
| 6 | **النص المشوّه يتراجع بأمان** | لا يرفع |

---

## 9. مقترح للنقل إلى Rust (غير مُلزِم)

| البند | القرار |
|---|---|
| المُنقّي | دالة واحدة `fn sanitize(input: &str) -> Result<String, SanitizeError>` |
| fail-closed | `?` في كل مسار كتابة تقرير |
| بلا مخرج خام | **لا تأخذ معامل `redact`** — هذا هو العقد |
| سجل النشاط المحلي | مسار منفصل مكتوب صراحةً، بلا منقٍّ |
| Idempotency | اختبار |
| الأرقام/التواريخ | اختبار (نفس `1.21.1` وISO-8601) |
| JSON | تكراري عبر `serde_json::Value` |
| **18 اختباراً** | انقلها كسلوك لا كنص مصدر |
| `tracing` | `foundations.md` §3.2 يختار `tracing` + `tracing-appender` — **طبّق المنقّي على spans القابلة للمشاركة فقط** |

> **⚠️ قرار مطلوب منك:** هل `tracing` يُكتب إلى ملف **غير منقّى** محلياً (المحاكاة لسجل النشاط)؟ هذا يوفّر حرية تشخيص كاملة. **أنصح بنعم.**

---

## 10. غير مُتحقَّق

| البند |
|---|---|
| لم أُشغّل أي اختبار |
| قائمة الحقول الـ13 المنقّاة بالضبط (اختبار 1) — **لم تُقرأ أجسام الاختبار** |
| `packaging/license/verify_public_release_license_packaging.py` — مُستدعى سلوكياً في الاختبار 18، **لم يُقرأ** |
| `app\licensing.py` (350 سطراً) — **لم يُقرأ**؛ تقرير الوكيل وصفه من filenames |
| هل المنقّي يغطي بيانات اعتماد `Homebrew`/`GitHub Actionsوغيرها |
