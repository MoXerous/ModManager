# behaviors/archive-safety.md — فك الأرشيف ومُنقّي المسارات

> **المرجع:** `D:\latest_project\` (تطبيق بايثون/PySide6، `master` @ `4f71f18`) · `app\services\safe_archive.py` (197 سطراً) · `tests\qa\test_safe_archive.py` (190)
> ⚠️ **المود باكات مدخل غير موثوق.** `AGENTS.md` §5.6 يفرض `enclosed_name()` وحدوداً على الحجم والعدد. **الكود القديم لا يستخدم `enclosed_name()`** — بل منقّياً خاصاً به. هذا موصوف حرفياً أدناه.

---

## 1. الحدود — `ArchiveLimits` / `DEFAULT_ARCHIVE_LIMITS`

`ArchiveLimits` صنف `@dataclass(frozen=True)` (الأسطر 26-34):

| الحد | القيمة | الهدف |
|---|---|---|
| `max_members` | **50,000** | عدد العناصر |
| `max_member_size` | **4 GiB** (`4 * 1024**3`) | حجم الملف الواحد |
| `max_total_uncompressed_size` | **32 GiB** (`32 * 1024**3`) | الحجم الكلي |
| `max_compression_ratio` | **500.0** | نسبة الضغط |
| `compression_ratio_min_size` | **1 MiB** | **النسبة لا تُطبَّق إلا لعنصر ≥ 1 MiB** |
| `max_name_length` | **1024** | طول الاسم |
| `max_path_depth` | **64** | عمق المسار |

---

## 2. `safe_archive_member_name(name, *, is_directory=None) -> str`

**قبل أي فحص:** `unicodedata.normalize("NFC", name.replace("\\", "/"))`.
(توحيد NFC + تحويل `\` إلى `/` هما أساس كشف تصادم Unicode وتوجيه Windows.)

### 2-1. الرفض بالترتيب

| # | الشرط | الرسالة |
|---|---|---|
| 1 | اسم فارغ | `"Archive contains an empty member name."` |
| 2 | `\x00` أو أي `ord(c) < 32` | `"Archive member contains control characters: …"` |
| 3 | `len > 1024` | `"Archive member name is too long: …"` |
| 4 | **مطلق**: `startswith("/")` **أو** `^[A-Za-z]:/` | `"Archive contains an unsafe absolute member path: …"` |
| 5 | مكوّن `.` أو `..` أو فارغ بعد التقسيم على `/` | `"Archive contains an unsafe member path: …"` |
| 6 | العمق `> 64` | `"Archive member path is too deep: …"` |
| 7 | مكوّن بمسافة بادئة/ لاحقة (`part != part.strip()`) أو ينتهي بـ`.` | `"Archive member has an ambiguous path component: …"` |
| 8 | **أي `:`** في مكوّن | `"Archive member contains a platform-unsafe colon: …"` |
| 9 | `part.split(".",1)[0].casefold()` ∈ الأسماء المحجوزة لويندوز: `aux, con, nul, prn, com1..com9, lpt1..lpt9` | `"Archive member uses a reserved platform name: …"` |

### 2-2. تغطية سيناريوهات `AGENTS.md` §5.6

| السيناريو | كيف يُرفض |
|---|---|
| `../evil.txt` | القاعدة 5 |
| `/absolute.txt` | القاعدة 4 |
| `C:/drive.txt` و`C:\drive.txt` | القاعدة 4 (بعد تحويل `\`←`/`) |
| `C:x` (نسبي-المحرك) | القاعدة 8 (النقطتان) |
| **UNC** `\\server\share\x` | ← `//server/share/x` ← `startswith("/")` ⇒ **القاعدة 4**. **لا فرع UNC صريح** |
| `folder//alias.txt` | القاعدة 5 (مكوّن فارغ) |
| `folder/./alias.txt` | القاعدة 5 |
| `folder/trailing. /file.txt` | القاعدة 7 |
| `folder/CON.txt` | القاعدة 9 |

> **مقارنة بـ`AGENTS.md` §5.6:** النص يطلب `enclosed_name()` من `zip`. الكود القديم **لا يستدعيها** — منقٍّ خاص أقوى فعلياً (يرفض الأسماء المحجوزة والتطبيع والأعمدة). **في Rust: `enclosed_name()` إلزامية + حدّد إن كانت تغطي ما سبق.**

---

## 3. `inspect_zip_members` — فحوص لكل عنصر

| الفحص | الشرط | الرسالة |
|---|---|---|
| إعادة فحص الطول والعمق | مقابل الحدود **الممرَّرة** | مطابقة §2 |
| **تصادم/تكرار** | `_member_collision_key = name.rstrip("/").casefold()` | `"Archive contains a duplicate or colliding member: …"` |
| **مُشفَّر** | `flag_bits & 0x1` | `"Archive contains an encrypted member: …"` |
| **رابط رمزي / ملف خاص** | `_is_link_or_special(info)`: **فقط** عند `info.create_system == 3` (Unix)؛ `mode = (external_attr >> 16) & 0xFFFF`؛ رفض إن `stat.S_IFMT(mode)` ∉ `{0, S_IFREG, S_IFDIR}` | `"Archive contains a link or special-file member: …"` |
| **أحجام سالبة** | `file_size < 0` أو `compress_size < 0` | `"Archive member has invalid size metadata: …"` |
| **تجاوز حجم العنصر** | `file_size > max_member_size` | `"Archive member size exceeds …"` |
| **تجاوز الحجم الكلي** | تراكمي `total_uncompressed_size > max_total_uncompressed_size` | `"Archive total uncompressed size exceeds …"` |
| **نسبة ضغط مشبوهة** | `file_size / compress_size > max_compression_ratio` (و `compress_size == 0` ⇒ مشبوهة)، **فقط** إذا `file_size >= compression_ratio_min_size` | `"Archive member has an unsafe compression ratio: …"` |

> **روابط ZIP الرمزية لا تُستخرَج أبداً** — لا شيء هنا يفكّ الضغط.
> **ثغرة:** روابط رمزية أُنشئت على ويندوز (`create_system != 3`) **لا تُكتشف** — الدالة تُعيد `False` بلا تعليق. **غير مُتحقَّق** إن كانت ثغرة حقيقية.

---

## 4. التصادمات Unicode والحالة

مفتاح التصادم `name.rstrip("/").casefold()` بعد تطبيع NFC يعني:

| الحالة | النتيجة |
|---|---|
| `Example.jar` + `example.jar` | تصادم |
| `mods/café.jar` (NFC) + `mods/café.jar` (NFD) | تصادم |
| `a/b.jar` + `a/b.jar/` (مجلد ونفسه ملف) | تصادم (بعد `rstrip("/")`) |

> **مُثبَت باختبارين:** `test_rejects_casefold_duplicate_member_paths` و`test_rejects_unicode_normalization_collisions`.

---

## 5. القراءة المحدودة

`read_limited_member(archive, inventory, name, *, max_bytes) -> bytes` (164):

| الخطوة | السلوك |
|---|---|
| `max_bytes` سالب | `ValueError` عادي — **ليس `SafeArchiveError`** ⚠ عدم اتساق |
| عنصر مجلد | خطأ |
| `file_size > max_bytes` | **رفض قبل الفتح** |
| القراءة | `max_bytes + 1` ثم **إعادة فحص الطول الفعلي** |

**سقوف JSON عند المستهلكين:**
- `MRPACK_INDEX_MAX_BYTES = 4 * 1024 * 1024` — `mrpack_service.py:28`
- `PROFILE_EXPORT_JSON_MAX_BYTES` — `profile_export.py`

---

## 6. من يستدعي ماذا

| المستهلك | الرموز | الأسطر |
|---|---|---|
| `app/services/artifact_verifier.py` | `SafeArchiveError` · `inspect_zip_archive` | 12 · 125 |
| `app/services/mrpack_service.py` | `inspect_zip_archive` · `read_limited_member` · `safe_archive_member_name` | 449 · 452 · 573 · 684 · 705 · 708 · 725 |
| `app/services/mrpack_installer.py` | `inspect_zip_archive` · `safe_archive_member_name` | 914 · 1466 · 1712 |
| `app/services/profile_export.py` | `inspect_zip_archive` · `read_limited_member` · `safe_archive_member_name` | 164 · 352 · 362 · 384 |

**مُثبَت:** `test_archive_consumers_do_not_keep_private_duplicate_member_path_helpers` يؤكد غياب أي helpers نسخ-خاصة (`_safe_zip_member_name` / `_safe_import_zip_member_name`) ووجود `inspect_zip_archive`.
⚠️ **هذا اختبار نص مصدر لا سلوك.**

---

## 7. فحوص إضافية على الملفات المنزَّلة

`artifact_verifier.py` يُضيف على `.jar/.mrpack/.zip`:

1. ملف عادي **غير رابط رمزي**
2. مطابقة حجم **بالضبط**
3. **رفض استجابة HTML** (نوع المحتوى أو بصمة البايتات)
4. التحقق من **SHA-1 و SHA-512**
5. `inspect_zip_archive` كامل + **رفض الأرشيف الفارغ**

> الترتيب: حجم قبل HTML قبل هاش. **HTML يُرفض قبل الهاش** — وهذا أذكى من الهاش وحده (صفحة خطأ 200 لها هاش مختلف).

---

## 8. الاختبارات

### 8-1. `SafeArchiveInventoryTests` (8) — `BytesIO` في الذاكرة، بلا قرص وشبكة

| الاختبار | ما يؤكده |
|---|---|
| `test_accepts_safe_members_and_reads_only_bounded_member` | الترتيب بالإدراج + قراءة محدودة |
| `test_rejects_unsafe_or_ambiguous_member_paths` | **7 أسماء** مرفوضة في assertion واحد |
| `test_rejects_casefold_duplicate_member_paths` | `"duplicate"` |
| `test_rejects_unicode_normalization_collisions` | **يثبت** خطوة NFC |
| `test_rejects_member_count_individual_and_total_size_limits` | حدود مخصّصة، 3 رسائل |
| `test_rejects_suspicious_compression_ratio_without_decompressing` | `max_compression_ratio=2.0`, `min_size=1` |
| `test_rejects_encrypted_and_symlink_like_members` | `S_IFLNK` + `flag_bits |= 0x1` |
| `test_bounded_read_rejects_member_before_decompression` | `"read limit"` |

### 8-2. `ArchiveConsumerLimitTests` (4) — قرص حقيقي

| الاختبار | ما يؤكّده |
|---|---|
| `test_artifact_verifier_applies_shared_compression_ratio_policy` | **يرث** `DEFAULT_ARCHIVE_LIMITS` |
| `test_mrpack_index_rejects_oversized_json_member` | سقف `MRPACK_INDEX_MAX_BYTES` |
| `test_profile_export_preview_rejects_oversized_json_member` | سقف `PROFILE_EXPORT_JSON_MAX_BYTES` |
| `test_archive_consumers_do_not_keep_private_duplicate_member_path_helpers` | ⚠ نص مصدر |

---

## 9. اقتراح للنقل إلى Rust (غير مُلزِم)

| البند | القرار المقترح |
|---|---|
| `safe_archive_member_name` | استبدلها بـ`zip::read::enclosed_name()` **وأبقِ** القواعد الإضافية (محجوزات ويندوز، NFC، المسافات، `:`) كطبقة فوقها |
| `read_limited_member` | اقرأ بـ`take(max_bytes + 1)` على `Read` ثم افحص الطول |
| نسبة الضغط | `compressed_size == 0` ⇒ رُفض دائماً عند `file_size > 0` |
| الرابط الرمزي | افحص **`unix_mode()`** من `zip` — وهو يقرأ البيانات حتى لو لم يكن `create_system == 3`، فيُغلق ثغرة ويندوز |
| التشفير | `zip` لا يكشفه مباشرة؛ اقرأ بتّات العلم العام من ترويسة الملف المحلي (إزاحة 6، بت 0) |
| الأرقام | كل الحدود **وحدات ثنائية** (`4 * 1024**3` = 4 GiB وليس 4 GB) |

## 10. غير مُتحقَّق

| البند |
|---|
| لم أُشغّل أي اختبار |
| ثغرة الروابط الرمزية على ويندوز — **مُقدَّرة من قراءة الكود** لا مُثبتة باختبار |
| هل `4 GiB` لعنصر واحد و`32 GiB` كلياً مناسبه لمود باك حقيقي — لم يُختبر بحزمة حقيقية |