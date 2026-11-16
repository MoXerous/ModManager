# ADR 0007 — النسخ الاحتياطي للنسخة: أثرٌ داخل المجلد، واستعادةٌ تُزيح ولا تَدهس

- **الحالة:** مقبول
- **التاريخ:** 2026-11-16
- **الشجرة المرجعية:** `E:\ModManager`، الفرع `feat/s6-backup-restore`
- **يتعلق بـ:** `docs/spec/features.md` (F-113 · F-114 · F-115 · F-118) · `docs/spec/behaviors/profiles.md` §5 · `docs/spec/behaviors/import-export.md` §7 · ADR 0002 (الأنواع المولَّدة) · ADR 0003 §5 (`jar.disabled` هو مصدر الحقيقة) · ADR 0006 §6 (سببٌ متوقَّع = نتيجة خطة، لا خطأ IPC)

## 1. السياق

S6 آخر شريحة تُنشئ **أثرًا جديدًا على القرص**: مجلد نسخة وبيانًا (manifest). والشرائح التالية (التثبيت الآلي للتحديثات، S11) تعتمد عليها اعتمادًا مباشرًا: لا تحديث بلا نسخة تُرجَع إليها.

التطبيق القديم ينفّذ السلوك على ثلاث وظائف بايثون: `profile_update_backup.py` (نسخٌ فقط)، `profile_backup_restore.py` (استعادة بثلاث طبقات حماية)، `profile_worlds.py` (معاينة قراءة فقط). والنسخة الجديدة لا تملك اليوم شيئًا منها:

- `core/src/modpack/export.rs:509-511` يذكر `.modrinth_bulk_downloader_profile_backups` و`..._restore_safety` **كأسماء استبعاد فقط**، ولا قارئ لهما.
- لا أمر IPC، ولا نوع DTO، ولا جدول قاعدة بيانات.
- `core/src/instances/remove/` هو النموذج القريب: خطة ⇒ تنفيذ، مجلد مؤرَّخ، `remove_manifest.json`، ورفضٌ صريح لكل حارس.

فالقرار المطلوب ليس «هل نضيف نسخًا احتياطيًّا»، بل: **أين يعيش الأثر، وما الذي يجعله قابلًا للاستعادة بأمان، وكيف نُثبت أننا نستعيد الشيء الصحيح.**

## 2. القرار

تُصبح النسخة الاحتياطية **مجلدًا داخل نسخة المستخدم** (`<content_root>/.modrinth_bulk_downloader_profile_backups/<stamp>/`)، تُنسخ إليه `mods/` و`config/` و`instance.lock.json` و`profile.json` و`installed_mods.json` (و`saves/` إن اختارها المستخدم صراحةً)، ويُكتب في نهايته `backup_manifest.json` سجلًّا **لكل ملف** (مساره، حجمه، `sha256`، مصدره، وقته، و`version_id` المقفول) — وهو الأثر الوحيد الذي يجعل المجلد «نسخة» لا «حطام نسخ». والاستعادة **خطة ثم تنفيذ**: تُبنى خطة تُزيح الوجهة القائمة إلى `.modrinth_bulk_downloader_restore_safety/<stamp>/current/` بنقلٍ ذرّي واحد، ثم **تنسخ** من النسخة (لا تنقل)، وتتحقّق من كل ملف أثناء نسخه؛ وفشلُ مجموعةٍ يُرجعها إلى نسخة الأمان ويُعلن النتيجة `partial` في `restore_manifest.json`. **ووجهة الاستعادة ليست من البيان:** كل هدف يأتي من `EntrySource` وحده (خريطة ثابتة إلى `content_root/<group>`)؛ ومسار البيان عرضٌ لا توجيه (§6-5). و`saves` **محجوبٌ افتراضيًّا** في الاتجاهين: لا يُنسخ ولا يُستعاد إلا بعلمٍ صريح في الطلب. ومعاينة العوالم (F-118) تعيد تعداد ما تحت `saves/` **بلا كتابة بايت واحد وبلا عبور رابط**. وأي انهيار بين الإزاحة والنسخ يترك أثرًا **يُقرأ ويُعلن** (`pending_restores`) لا صمتًا (§6-6).

## 3. المفردات: `instance` لا `profile`

الكلمة المعتمدة في كل artefact جديد — الأنواع، الدوال، الأوامر، حقول DTO، مفاتيح i18n، `CONTEXT.md` — هي **instance**. الكلمة القديمة `profile` تبقى حيث تكون **اسمًا على القرص أو في مخطط** لا نملك تغييره:

| الموضع | الإبقاء | السبب |
|---|---|---|
| `.modrinth_bulk_downloader_profile_backups` | يُبقى حرفيًّا | سطح توافق: مجلدات قد تكون موجودة من التطبيق القديم، وقائمة استبعاد التصدير تعرفه بالفعل |
| `.modrinth_bulk_downloader_restore_safety` | يُبقى حرفيًّا | نفس السبب + `remove` سبقها إلى هذا القرار |
| `profile.json` | يُبقى | ملف محمول باسمه، تكتبه `core/src/instances/metadata.rs` |
| جدول `profiles` في قاعدة البيانات القديمة | يُبقى | ملكية `core/src/legacy/` وحدها؛ لا نقرأه في هذه الشريحة |

أي معرّف جديد يقول `profile` (نوعًا أو حقلًا أو مفتاح ترجمة) فهو **خطأ مراجعة**. التعيين القديم ↔ الجديد موثَّق في `core/src/legacy/`، ولا يُعاد اختراعه هنا.

## 4. أثر النسخة على القرص

`content_root` = `minecraft_root` الصفّ (هو `path` نفسه حين لا يوجد غلاف `.minecraft`، كما في `detect_instance_roots`). نضع الأثر في مجلد المحتوى لا في غلاف Prism، لأن كل المسارات المستعادة (`mods/`، `config/`، `saves/`، القفل) تُقاس منه، فيبقى **جذرٌ واحد** لكل حراسة.

```
<content_root>/
  .modrinth_bulk_downloader_profile_backups/
    2026-11-16T09_41_12Z/
      backup_manifest.json
      mods/…              نسخة مرآة لما كان في mods/
      config/…
      saves/…             فقط مع include_saves
      instance.lock.json
      profile.json
      installed_mods.json (إن وُجد)
  .modrinth_bulk_downloader_restore_safety/
    2026-11-16T09_50_03Z/
      current/mods/…      الوجهة القائمة، مُزاحة لا مُتلفة
      current/config/…
      failed_restore/…    ما فشل نسخه، للمراجعة اليدوية
      restore_manifest.json
```

`<stamp>` = `utc_timestamp` بصيغة `YYYY-MM-DDTHH_MM_SSZ` مع `-1`، `-2` عند التصادم (`unique_path`)؛ فالنسختان في الثانية نفسها لا تتصادمان.

**ما يُنسخ ومتى:**

| المصدر | يُنسخ دائمًا؟ | ملاحظة |
|---|---|---|
| `mods/` | نعم | جوهر F-113. غيابه ليس خطأً: يُسجَّل `skipped` بسبب `source-missing` |
| `config/` | نعم | الفئة الثانية في صفّ F-113 |
| `instance.lock.json` | إن وُجد | بديل `installed_mods.json` في تطبيقنا: بدونه تصبح كل المودات المستعادة «خارجية» (F-097) |
| `profile.json` | إن وُجد، وإلا **يُولَّد** من الصفّ | لقطة هوية داخل النسخة فقط (`status: "generated"`) — سلوك التطبيق القديم نفسه |
| `installed_mods.json` | إن وُجد | ملف ملكية قديم، يُنسخ ويُستعاد **كتلة معتمة** بلا تفسير؛ تفسيره ملك F-181 |
| `saves/` | **لا** | F-115: بعلم `include_saves` فقط |

**ما يُستبعد دائمًا:** `.modrinth_bulk_downloader_profile_backups` نفسها (نسخة لا تحتوي نسخة)، `.modrinth_bulk_downloader_restore_safety`، `.modrinth_bulk_downloader_removed_content`، `.modrinth_bulk_downloader_backup` (مجلد نسخ التحديث القديم)، `.mm-staging` — بأسمائها في **أي عمق**، بنفس منطق `_copytree_ignore` القديم و`in_backup_folder` في `remove`.

**ما يُتخطّى ولا يُتبَع:** أي رابط رمزي أو نقطة إعادة تحليل داخل المصدر. لا ننسخ الرابط (انظر الانحراف رقم 1)، ولا نعبره. ونوع الملف يُقرَّر عبر `symlink_metadata`، لا `metadata`، فلا يُفتح ما يشير إليه الرابط أصلًا.

**ولا نكتب في النسخة الحيّة إطلاقًا:** النسخ قراءة من المصدر وكتابة في مجلد جديد؛ لا حذف، ولا نقل، ولا إعادة تسمية، ولا `jar.disabled` يُقلَب. اختبار الشريحة يُقارن شجرة النسخة الحيّة قبل/بعد بمقارنة `(path, len, modified)` كما في `instances::snapshot_tree`.

## 5. صيغة البيان

`core/src/instances/backup/manifest.rs`، وبأسلوب `lock.rs`: مفكِّك متسامح مع الحقول الغائبة، ورافض لمخطط أحدث.

```rust
pub const BACKUP_SCHEMA: &str = "mm.instance_backup.v1";

pub struct BackupManifest {
    pub schema: String,
    pub created_at: String,              // UTC ISO-8601 ثانية
    pub instance_id: String,             // معرّف الصفّ: هو ما يُثبت «النسخة الصحيحة»
    pub instance_name: String,           // للعرض فقط
    pub minecraft_version: Option<String>,
    pub loader: Option<String>,
    pub app_version: String,             // من mm_core::app_info()
    pub include_saves: bool,
    pub entries_sha256: String,          // بصمة كتلة entries (أدناه)
    pub entries: Vec<BackupEntry>,
}

pub struct BackupEntry {
    pub source: EntrySource,   // profile_json | installed_mods | mods | config | saves | lock
    pub path: String,          // نسبة إلى content_root، بفواصل `/`، مثل "mods/sodium.jar"
    pub kind: EntryKind,       // file | dir
    pub size: u64,
    pub sha256: String,        // فارغة للمجلدات وللمدخلات المتخطّاة
    pub version_id: String,    // من القفل لذلك الملف؛ "" لغير المُدار
    pub status: EntryStatus,   // copied | generated | skipped
    pub reason_code: String,   // رمز ثابت عند skip (source-missing، symlink، …)
}
```

| الحقل | لماذا هو موجود |
|---|---|
| `instance_id` | ليس الاسم (قابل للتعديل) ولا المسار (قابل للنقل): المعرّف هو ما يربط النسخة بالصفّ الذي ستُستعاد عليه |
| `path` | يُعيد بناء الشجرة، وهو **مُدخل غير موثوق**؛ عند الاستعادة **عرضٌ فقط** ولا يُشتقّ منه هدف (§6-5) |
| `size` + `sha256` | إثبات أن البايتات التي نستعيدها هي البايتات التي نسخناها |
| `source` | يجمع المجموعة بلا تحليل نصّ المسار، ويسمّي ما نسخناه (نفس معنى `label`/`source` في بيان التطبيق القديم) |
| `modified_at` | **أُزيل (انحراف ٩):** لا شيء يقرأه، والنسخ لا يستطيع إعادة كتابته |
| `version_id` | **حقل الإصدار**: يسمح بتقرير «هذه النسخة تحوي ٤٢ ملفًا مُدارًا، ٣ منها لم تعد في القفل الحالي» بلا فتح أي jar |
| `status` + `reason_code` | النسخة التي تُسقط رابطًا أو مجلدًا بصمت تكذب بالنقصان؛ البيان يقول ما ليس فيه ولماذا |

**البصمة `entries_sha256`:** `SHA-256` على تمثيل JSON المضغوط لـ`entries` بترتيبها وبترتيب حقول البنية (نفس التعريف يُنتج نفس البايتات دائمًا). تُحسب عند الكتابة، وتُعاد عند القراءة؛ اختلافها ⇒ `ManifestError::DigestMismatch`. **وهي ليست توقيعًا ولا حدود أمان:** من يملك الكتابة في مجلد النسخة يستطيع إعادة كتابة البيان والملفات معًا. فائدتها منعُ التلف والقصّ والتحرير العرضي — أما الحدود الحقيقية فهي §6.

**الكتابة ذرّية:** البيان يُكتب بـ`write_atomic_unique` (لا `.part` ثابت، فلا يُقفل الكتابة التالية بقايا انهيار) داخل `spawn_blocking`. **ولا يُعتبر المجلد نسخةً إلا بوجوده**: مجلد بلا بيان = `manifest_state: "missing"`، ترفضه الاستعادة (`BackupMissing`) ولا يظهر كخيار صالح في الواجهة. النسخ المتوقّف في منتصفه يترك مجلدًا مرفوضًا صراحةً، لا نصف نسخة موثوقة.

**القراءة:** حدّ ٤ MiB كما في `metadata.rs`، ورفض بايت NUL، و`schema` يُقارن: بادئة `mm.instance_backup.v` ورقم > ١ ⇒ `NewerSchema` (يُترجم إلى `ErrorCode::NewerSchema`)، وبادئة أخرى ⇒ `Invalid`.

**نسخ التطبيق القديم: تُعرض ولا تُستعاد.** التطبيق القديم كتب `"schema": "modrinth_bulk_downloader.profile_update_backup.v1"`، وقارئنا يرفض كل بادئة غير `mm.instance_backup.v1`. فمجلد النسخة القديم يبقى في مكانه (اسم المجلد محفوظ للتوافق على القرص) ويظهر في القائمة بحالة `manifest_state: "legacy"` — **مرفوض للاستعادة** برمز `BackupMissing` عند اختياره، لا محوَلًا ولا مُصلَحًا ولا مُعاد كتابته. الاسم محفوظ، والمحتوى ليس عقدًا: لا نقرأ `entries` القديمة (صفوف مجموعة بلا هاشات ولا مقاسات) ولا نخمّن ما فيها.

## 6. دلالة الاستعادة

### 6-1. القاعدة الفاصلة: خطأ على مستوى الطلب، وحكم على مستوى المدخل

| النوع | الشكل | أمثلة |
|---|---|---|
| **رفض للطلب كله** | `IpcError { code, message }` | مجلد النسخة خارج مجلد النسخ، بيان لنسخة أخرى، بيان تغيّر بعد المعاينة، كلمة تأكيد خاطئة |
| **حكم على مدخل** | صفٌّ في الخطة `allowed: false` + `reasonCode` | `saves-blocked-by-default`، `entry-is-link`، `safety-exists`، `target-under-link` |

هذا هو التقسيم نفسه في ADR 0006 §6، ولذلك تُترجم الواجهة الرمز ولا تقرأ نصًّا.

### 6-2. ثلاث طبقات حماية قبل أي كتابة

1. **الاحتواء:** مسار مجلد النسخة بعد `resolve` يجب أن يقع داخل `<content_root>/.modrinth_bulk_downloader_profile_backups`. (نفس حارس التطبيق القديم `profile_backup_restore.py:116`.)
2. **الهوية:** `manifest.instance_id` يجب أن يساوي `id` الصفّ المسجَّل للجذر المستهدف. الاسم والمسار لا يُقبلان بديلًا: الاسم يُعدَّل، والمسار ينتقل. عدم التطابق ⇒ `ErrorCode::RestoreMismatch`.
3. **الحداثة:** الطلب يحمل `expected_manifest_sha256` (بصمة بايتات البيان التي عرضتها الخطة). التنفيذ يُعيد حسابها؛ اختلافها ⇒ `ErrorCode::BackupChanged` — نفس نمط `expectedLockSha512` في التصدير و`packSha512` في التثبيت (روح F-121: خطة تتقادم).

### 6-3. الوجهة القائمة: تُزاح، لا تُدهس ولا تُحذف

لكل مجموعة (`mods/`، `config/`، …) إن كانت الوجهة موجودة:

```
rename(target, <safety>/current/<group>)      // نقلة واحدة على القرص نفسه
copy(<backup>/<group>, target)                // نسخ، لا نقل: النسخة تبقى قابلة للاستعادة مرة أخرى
```

- إن كان مسار الأمان موجودًا مسبقًا ⇒ يُرفض ذلك المدخل (`safety-exists`) ولا يُستبدل شيء.
- **مسار الأمان نفسه هدفُ كتابة، فيُثبَّت على الجذر:** `safety_dir` و`safety_path` يُشترطان داخل `content_root` بقاعدة المسارات نفسها (`starts_with` على مكوّنات مُطبَّعة، و`..` تُحسب لا تُتبَع) — في التخطيط (`safety-outside` كحكم مدخل) وفي التنفيذ: خطةٌ تشير إلى مجلد أمان خارج النسخة تُرفض بـ`RestoreError::SafetyOutside` **قبل أول إعادة تسمية وقبل إنشاء أي مجلد**، وإلا نقلت `mods/` إلى أي مكان يسمّيه من صنع الخطة (النواة تُستدعى من CLI، ولاحقًا من تثبيت التحديث التلقائي).
- **الحارس من الجذر المستحقّ نزولًا، لا من الملف صعودًا:** يُفحص المسار من `content_root` إلى الهدف بـ`guard::link_at_or_inside(content_root, target)` (و`guard::link_inside` حيث لا يُحكم على الجذر نفسه)، وإلا `target-under-link`. **ولا يُستخدم `guard::link_in_ancestors(target)` هنا**: فهو يحكم ما **فوق** الجذر المستحقّ — مجلد منزل مربوط، أو `/var` على macOS — فيرفض استعادةً مشروعة على جهاز سليم. الجذر يضمنه المستخدم، وما تحته فقط ملكنا.
- بعد النسخ تُقارن بصمة كل ملف بالبيان أثناء النسخ نفسه (مرور واحد: نُهشِّر ما نكتبه). اختلاف ⇒ **تراجع تلك المجموعة**: يُنقل الهدف الجزئي إلى `<safety>/failed_restore/<group>` وتُعاد نسخة الأمان بـ`rename` واحد. لا تُترك شجرة نصف مستعادة.
- **الترتيب مقصود:** `mods/` ← `config/` ← (`saves/` و`installed_mods.json`) ← `instance.lock.json` **أخيرًا**. القفل آخرًا لأنه ادّعاء ملكية: استعادته قبل الملفات تجعله يشهد على ملفات لم تصل بعد. وإن فشلت مجموعة `mods/` فلا يُستعاد القفل، ويُسجَّل `dependency-failed`.
- **`profile.json` لا يُستعاد أبدًا** (انحراف مسجَّل، §11): الصفّ في SQLite هو الحقيقة، والملف مرآة مشتقّة قابلة لإعادة التوليد. نسخُه من الأقدم يُعيد ميتافيزقا تكذب على الصفّ.

### 6-4. الحالات النهائية وما يُبلَّغ

`RestoreOutcome { Complete | Partial | Cancelled }`، ولكل مجموعة `state`:

`restored` · `failed` · `rolled-back` · `rollback-failed` · `cancelled` · `dependency-failed`

- **`rollback-failed` ليس تزيّدًا:** تراجع المجموعة يُنفَّذ بإعادتَي تسمية (الهدف الجزئي إلى `failed_restore/<group>`، ثم نسخة الأمان إلى الهدف)، وكل نتيجة تُفحص. مجموعةٌ فشلت **وتعذّر إرجاع نسختها** ليست `rolled-back` وليست `failed` فحسب: هدفها مفقود أو نصف مكتوب، فالصفّ يقول ذلك و`outcome` يبقى `Partial`، والواجهة تُرسل المستخدم إلى `safety_path`. الإبلاغ بـ`rolled-back` عن مجموعة لم تُرجَع هو الكذبة الوحيدة التي لا يحتملها هذا التصميم (انحراف ١٠ عن قائمة المواصفة ذات الخمس حالات). وإلغاءُ مجموعةٍ **لم يكن لها هدف سابق** يبقى `cancelled` (لا شيء ليعود، والشجرة النصفية أُزيحت) — إيقافُ المستخدم ليس فشلًا.

- المدخل المحجوب في الخطة **لا يدخل التشغيل أصلًا**؛ فحالته حالة خطة (`blocked` + `reasonCode`) لا حالة تشغيل، ويُسجَّل في `restore_manifest.json` كذلك — كي لا يقرأ المستخدم تقريرًا يخفي ما لم يُستعد.
- الفشل **معزول بمجموعة**: مجموعة تفشل لا تُسقط البقية (سلوك التطبيق القديم)، لكن `outcome` يصبح `Partial` لا `Complete`، وتُعرض للمستخدم مسارات مجلد الأمان وما لم يُستعد. **لا حالة نصفية صامتة.**
- الإلغاء يُفحص بين المجموعات **وبين الملفات داخل المجموعة**؛ المجموعة التي تُلغى في منتصفها تُرجَع كاملة إلى نسخة الأمان وتُسجَّل `cancelled`. (التطبيق القديم كان يفحص بين المدخلات فقط، فكان الإلغاء قد يترك مجموعة نصفها مستعاد.)
- `restore_manifest.json` يُكتب في نهاية كل تشغيل — بما فيه التشغيل الجزئي والملغى — بـ`write_atomic_unique`، ويحمل: `schema` (`mm.instance_restore.v1`)، `created_at`، `backup_dir`، `safety_dir`، `manifest_sha256` (بصمة البيان المستعاد)، `outcome`، وصفوف المجموعات بحالاتها وأسبابها، **والمداخل المحجوبة كما حجبها التخطيط**.
- **لا سجل عمليات في `journal`:** البيان نفسه هو السجل. مجلد النسخة يجب أن يبقى مقروءًا بعد إعادة تثبيت التطبيق أو على جهاز آخر، وسجلّ العمليات خاصٌّ بالجهاز (`paths::operations_dir`) ولا يحمل ما يكفي لإعادة بناء الشجرة. ولذلك لا يُضاف `OperationKind` جديد في هذه الشريحة.

### 6-5. وجهة الاستعادة تُشتقّ من `EntrySource`، لا من البيان

البيان **مُدخل غير موثوق**: قد يكون محرَّرًا بيد، أو منسوخًا من نسخة أخرى. لذلك لا يُشتقّ أي مسار كتابة من حقل `path`:

```rust
pub enum EntrySource { ProfileJson, InstalledMods, Mods, Config, Saves, Lock }
```

| `EntrySource` | هدف الاستعادة (ثابت) | نوع |
|---|---|---|
| `Mods` | `<content_root>/mods` | مجلد |
| `Config` | `<content_root>/config` | مجلد |
| `Saves` | `<content_root>/saves` | مجلد |
| `Lock` | `<content_root>/instance.lock.json` | ملف |
| `InstalledMods` | `<content_root>/installed_mods.json` | ملف |
| `ProfileJson` | `<content_root>/profile.json` | ملف (لا يُستعاد أصلًا، §6-3) |

- **الملفات تُعدّ من القرص داخل مجموعة النسخة، لا من البيان:** يُنمشى `<backup>/<group>/` (بـ`symlink_metadata`، بلا عبور رابط) ويُنسخ كل ابن إلى `<target>/<نسبته من المجموعة>`. فالاسم الوحيد الذي يوجّه الكتابة هو اسم نسبي أنتجته *قراءتنا* لشجرة النسخة.
- **`path` عرضٌ وتحقّق، لا توجيه:** يُقارَن `path` بمجموعته؛ ويُعدّ مدخلًا **محجوبًا** (`entry-outside-group`) كلُّ مدخل لا يستقرّ مساره المسجَّل تحت مجلد مجموعته: مكوّن `..`، مسار مطلق، بادئة قرص (`C:`)، أو مكوّن رابط/نقطة إعادة تحليل. الحجب في `build_restore_plan` **ويُعاد في `execute_restore`** (خطة قد تُصنع بيد، والقرص قد يتغيّر بينهما) — بنفس منطق `backup-outside` في `remove`.
- **حارس الرابط على المسار الكامل:** بعد تركيب الهدف، يُحكم بالجذر المستحقّ نزولًا (`link_at_or_inside(content_root, target)`) — انظر §6-3.

### 6-6. الانهيار بين الإزاحة والنسخ: أثرٌ يُقرأ ويُعلن

الاستعادة تُزيح الوجهة (`rename`) ثم تنسخ، ولا يُكتب `restore_manifest.json` إلا في النهاية. فانهيارٌ بين الخطوتين يترك نسخةً **بلا `mods/`** وقفلًا قديمًا يدّعي ملفات لم تعد. هذا خطرٌ حقيقي، ولا يُعالَج ببيانٍ لم يُكتب:

- **سطح استرداد للقراءة فقط:** `pending_restores(root) -> Vec<PendingRestore>` يتعدّد `<content_root>/.modrinth_bulk_downloader_restore_safety/*/`، ويُعيد كل `<stamp>` **بلا `restore_manifest.json`** مع: `safety_dir`، `has_current` (هل `current/` غير فارغ)، والمجموعات الموجودة فيه. لا كتابة، ولا حذف، ولا إزاحة تلقائية، ولا استنتاج «ما الذي كان» من أثر ناقص.
- **يُعرض كما تُعرض استعادة الحزم المعلَّقة:** لافتة في `InstanceDetails` (على غرار `PackRecoveryBanner`) تقول: «استعادة لم تكتمل، والوجهة السابقة في `<safety>`» مع زرّ يفتح المجلد. **ولا زرّ «إصلاح تلقائي»**: الاختيار بين إعادة الوجهة السابقة وإعادة المحاولة قرار المستخدم.
- **الترتيب يحدّ من الضرر ولا يمنعه:** الكتابة في `restore_manifest.json` بـ`write_atomic_unique` تجعل «بيان موجود» تعني «تشغيل انتهى»؛ ولذلك **غيابه هو الإشارة**، لا محتواه.
- **حدود صريحة في القسم 13:** لا ضمان ذرّي عبر مجموعتين، ولا استعادة تلقائية بعد انهيار — النسخة الاحتياطية نفسها هي طريق الرجوع، واللافتة تقول أين هي.


## 7. `saves`: بعلمٍ صريح، محجوبٌ افتراضيًّا

«محجوب افتراضيًّا» في هذه الواجهة البرمجية يعني ثلاثة أشياء ملموسة، لا نيّةً في التعليقات:

1. **في الطلب:** `BackupRequest { instanceRoot, #[serde(default)] includeSaves: bool }` — الغياب يعني `false`، ولا يُستنتج من البيان ولا من وجود مجلد `saves/`.
2. **في نوع النواة:** `build_backup_plan(.., include_saves: bool, ..)` بارامتر `bool` لا `Option<bool>`؛ فلا معنى ثالثًا لـ«لم يُحدَّد» يمكن أن يُقرأ على أنه «نعم» في يومٍ ما.
3. **في الاستعادة:** كون البيان يقول `include_saves: true` **لا يبيح** استعادة `saves`. الطلب يحمل العلم مستقلًّا، والبيان يسجّل ما نُسخ لا ما يجوز استعادته. النسخة التي فيها `saves/` وطلبُ استعادتها بلا العلم تُنتج مدخلًا **محجوبًا** (`allowed: false`، `reasonCode: "saves-blocked-by-default"`) يظهر في المعاينة — لا حذفًا صامتًا ولا خطأً.

وإن كان `saves` رابطًا رمزيًّا ⇒ `saves-is-link`؛ ولا نُنشئ `saves/` إن لم تكن موجودة.

## 8. معاينة العوالم (F-118): قراءة فقط، وما لا تكتبه أبدًا

`preview_worlds(content_root) -> WorldsPreview`:

- تعداد **الأبناء المباشرين** لـ`<content_root>/saves` فقط. لا مشي متكرّر.
- تُتخطّى الأسماء التي تبدأ بنقطة (وهكذا تُتخطّى مجلداتنا المخبأة بلا قائمة استبعاد ثانية). **الاستثناء من العدّ مطبَّق أيضًا على `datapacks/`:** الأبناء ذوو النقطة لا يُعدّون في `datapackCount`.
- **الترتيب أبجديّ لا يعتمد على حالة الأحرف** (`sort_by_key(name.to_lowercase())`)، فالفرز في نظامين لا يُنتج ترتيبين.
- **ما ليس مجلدًا يُتخطّى بعد فحصه كرابط:** الملفات العادية في `saves/` لا تُدرَج في القائمة (نفس سلوك التطبيق القديم).
- **الرابط الرمزي يُدرَج ولا يُفحَص:** يُقرأ بـ`symlink_metadata`، ويُعطى `status: "symlink"` و`datapackCount: 0` و`hasLevelDat: false`، ولا يُفتح ولا يُعدّ ما تحته — قراءة معاينة لا تعبر رابطًا لا يملكه المستخدم.
- لكل مجلد: هل `level.dat` **ملف عادي** (بـ`symlink_metadata`، فالرابط ليس ملفًّا)، وعدد أبناء `datapacks/` المباشرين إن كان مجلدًا غير رابط (وإلا فـ`0`).
- `WorldStatus { Detected | MissingLevelDat | Symlink }` كرموز، والتفسير في مفاتيح i18n.

**ما لا تفعله هذه المعاينة، والاختبار يُثبته:** لا `create_dir`، ولا كتابة، ولا إعادة تسمية، ولا نسخ، ولا فحص محتوى `level.dat`، ولا عبور رابط. الاختبار يُقارن شجرة `saves/` قبل/بعد بايتًا ووقتًا. أما **استيراد** عالم فقرة أخرى مؤجَّلة بقرارها (F-119 = Defer) وليست جزءًا من هذه الشريحة.

## 9. الحدود البرمجية

### 9-1. النواة: `core/src/instances/backup/`

```
mod.rs        // الثوابت، الخطة، التنفيذ، التعداد، الحالات
manifest.rs   // BackupManifest + قراءة/كتابة/بصمة
restore.rs    // RestorePlan + execute_restore
worlds.rs     // preview_worlds
tests.rs      // كل اختبارات الشريحة
```

```rust
pub const BACKUP_DIRNAME: &str = ".modrinth_bulk_downloader_profile_backups";
pub const SAFETY_DIRNAME: &str = ".modrinth_bulk_downloader_restore_safety";
pub const BACKUP_MANIFEST_NAME: &str = "backup_manifest.json";
pub const RESTORE_MANIFEST_NAME: &str = "restore_manifest.json";
pub const RESTORE_SCHEMA: &str = "mm.instance_restore.v1";
pub const CONFIRMATION_WORD: &str = "RESTORE";

pub struct BackupSubject { pub instance_id: String, pub name: String,
                           pub minecraft_version: Option<String>, pub loader: Option<String> }

pub fn build_backup_plan(root: &Path, subject: &BackupSubject,
                         include_saves: bool, now: SystemTime) -> Result<BackupPlan, BackupError>;
pub fn execute_backup(plan: &BackupPlan, cancel: &AtomicBool,
                      on_progress: &mut dyn FnMut(BackupProgress))
                      -> Result<BackupRun, BackupError>;

pub fn list_backups(root: &Path, instance_id: &str) -> Vec<BackupCandidate>;
pub fn read_manifest(path: &Path) -> Result<BackupManifest, ManifestError>;
pub fn manifest_digest(path: &Path) -> Result<String, ManifestError>;

pub fn build_restore_plan(root: &Path, subject: &BackupSubject, backup_dir: &Path,
                          include_saves: bool, now: SystemTime)
                          -> Result<RestorePlan, RestoreError>;
pub fn execute_restore(plan: &RestorePlan, confirmation: &str, cancel: &AtomicBool,
                       on_progress: &mut dyn FnMut(RestoreProgress))
                       -> Result<RestoreRun, RestoreError>;

pub fn pending_restores(root: &Path) -> Vec<PendingRestore>;

pub fn preview_worlds(root: &Path) -> WorldsPreview;
```

`EntrySource::target_of(content_root) -> PathBuf` هي الخريطة الوحيدة المسموح بها (§6-5)؛ لا دالة تأخذ مسارًا من البيان وتُعيد هدفًا.

النواة لا تعرف Tauri ولا قاعدة البيانات: `BackupSubject` يُبنى في طبقة IPC من `InstanceRow`. الاختبارات تمرّر `now: SystemTime` صريحًا (كما في `build_remove_preview`) فيصير اسم المجلد والطوابع قابلة للاختبار.

**أدوات مشتركة:** `utc_timestamp` و`backup_stamp` و`unique_path` خاصةٌ اليوم بـ`instances/remove/mod.rs`؛ تُنقل إلى `instances/stamp.rs` بـ`pub(crate)` بلا تغيير سلوك (نقلٌ ميكانيكي، واختباراتها القائمة تبقى شاهدة). التكرار مرفوض: نسختان من تنسيق الوقت تعنيان مجلدين لا يفرز بعضهما.

`BackupError`: `RootMissing` · `RootIsLink` · `Io` · `Manifest(ManifestError)` · `Cancelled`.
`RestoreError`: `BackupOutside` · `BackupMissing` · `InstanceMismatch` · `ManifestChanged` · `ConfirmationRequired` · `Manifest(ManifestError)` · `Io`.

### 9-2. IPC: `src-tauri/src/commands/backup.rs` (+ `dto.rs`)

| الأمر | يعيد | يكتب؟ |
|---|---|---|
| `list_instance_backups { instanceRoot }` | `BackupListDto` | لا |
| `execute_instance_backup { req: BackupRequest, onEvent: Channel<BackupEventDto> }` | `BackupResultDto` | نعم |
| `cancel_instance_transfer { instanceRoot }` | `bool` | لا |
| `plan_instance_restore { req: RestorePreviewRequest }` | `RestorePlanDto` | لا |
| `execute_instance_restore { req: RestoreExecuteRequest, onEvent: Channel<RestoreEventDto> }` | `RestoreRunDto` | نعم |
| `preview_instance_worlds { instanceRoot }` | `WorldsPreviewDto` | لا |
| `pending_instance_restores { instanceRoot }` | `Vec<PendingRestoreDto>` | لا |

كل الأوامر تمرّ بـ`registered_root` (نفس بوابة `pack`: لا نسخ ولا استعادة لمجلد غير مسجَّل)، وتأخذ `apply_lock_for(root)` كي لا يتزاحم نسخٌ مع تطبيق حزمة على الشجرة نفسها.

**الإلغاء حقيقي لا صُوريًّا:** `cancel_instance_transfer` يُخاطِب **علم النسخ الجاري على ذلك الجذر**، لا علمًا محليًّا: `execute_*` يُسجّل العلم في `AppState::begin_transfer(&root)` **قبل بناء الخطة**، و`TransferGuard` يُسقط التسجيل على كل مخرج، و`cancel_transfer` يقلب العلم نفسه. الخريطة بمفتاح الجذر لا خانةٌ واحدة: قفل التطبيق لكل جذر يسمح بنسخين متوازيين على نسختين، وخانةٌ واحدة تجعل إلغاء الثانية يصل إلى الأولى. والجذر الذي له علم مُسجَّل **يتبنّاه** ولا يُستبدل، كي لا يضيع إلغاءٌ وصل بين النقرة وتسجيل التشغيل.

DTOs (كلها `#[serde(rename_all = "camelCase")] #[ts(export)]`):

```
BackupListDto { backupRoot, backups: Vec<BackupCandidateDto> }
BackupCandidateDto { stamp, createdAt, includeSaves, manifestState, instanceMatches,
                     entryCount: number, totalBytes: number }
BackupRequest { instanceRoot, #[ts(optional)] includeSaves }
BackupResultDto { backupDir, createdAt, manifestPath, manifestSha256, includeSaves,
                  copiedFiles: number, copiedBytes: number, skipped: Vec<BackupSkipDto> }
BackupSkipDto { source, path, reasonCode }
RestorePreviewRequest { instanceRoot, backupDir, #[ts(optional)] includeSaves }
RestorePlanDto { backupDir, createdAt, safetyDir, includeSaves, manifestSha256,
                 allowed: Vec<RestoreEntryDto>, blocked: Vec<RestoreEntryDto> }
RestoreExecuteRequest { instanceRoot, backupDir, includeSaves,
                        expectedManifestSha256, confirmation }
RestoreEntryDto { source, backupPath, targetPath, safetyPath, fileCount: number,
                  totalBytes: number, allowed, reasonCode }
RestoreRunDto { outcome, safetyDir, manifestPath,
                entries: Vec<RestoreResultEntryDto>, blocked: Vec<RestoreEntryDto> }
RestoreResultEntryDto { source, backupPath, targetPath, safetyPath, state, reasonCode,
                        fileCount: number, totalBytes: number }
WorldsPreviewDto { savesDir, savesExists, worlds: Vec<WorldDto>, messageCode }
WorldDto { name, status, datapackCount: number, hasLevelDat, messageCode }
PendingRestoreDto { stamp, safetyDir, hasCurrent, groups: Vec<String>, messageCode }
```

التقدّم عبر القناة نفسها ومُقيَّد بـ`progress::ProgressThrottle` (١٠٠ ms، والحدث النهائي لا يُسقَط): `BackupEventDto { phase, copiedFiles, totalFiles, copiedBytes, totalBytes, terminal }`، حيث `phase` **مفتاح i18n** (`backup.phase.*`، `restore.phase.*`) لا كلمة عرض — كما في `pack.phase.*`.

أكواد خطأ جديدة في `ErrorCode` (والواجهة تترجمها): **`RestoreMismatch`** و**`BackupChanged`**. وتُعاد الاستعانة بـ`Io` · `NotFound` · `InvalidInput` (كلمة التأكيد) · `LinkOrReparse` · `NewerSchema` · `Cancelled` · `Internal`. مفاتيح `error.restoreMismatch` و`error.backupChanged` في `ar.json` و`en.json` (وإلا فشل `pnpm i18n:check`).

### 9-3. الواجهة

`src/app/pages/InstanceDetails.tsx` قسمان جديدان: «النسخ الاحتياطي» (قائمة، إنشاء بزر واحد مع خانة `saves` **غير مفعّلة افتراضيًّا**، واستعادة بحوار خطة على غرار `RemoveDialog.tsx` ينتهي بكتابة `RESTORE`) و«العوالم» (جدول قراءة فقط، بلا أزرار — كما في `ProfileWorldsDialog`). **وزرّ إيقاف** أثناء النسخ وأثناء الاستعادة ينادي `cancel_instance_transfer` (F-114). كل النصوص من `src/i18n/{ar,en}.json`، و`src/lib/ipc.ts` يحصل على دوال هذا القسم، والمجلّد `src/bindings/` يُعاد توليده بـ`cargo test --workspace`.

## 10. الاختبارات (الأسماء تحمل معرّف المواصفة — `AGENTS.md` §6)

`core/src/instances/backup/tests.rs`:

- `spec_f113_backup_copies_mods_and_config_and_writes_a_manifest`
- `spec_f113_backup_never_writes_into_the_live_instance` (مقارنة شجرة قبل/بعد)
- `spec_f113_backup_excludes_its_own_folders_and_removed_content`
- `spec_f113_backup_skips_a_missing_folder_with_a_reason_code`
- `spec_f113_backup_skips_a_symlink_and_never_follows_it`
- `spec_f113_manifest_is_written_atomically_and_leaves_no_temp` (رابط صلب يشهد أن الكتابة إعادةُ تسمية لا قصٌّ في المكان)
- `spec_f113_manifest_records_no_mtime_the_copy_cannot_restore`
- `spec_f114_restore_refuses_a_folder_outside_the_backup_root`
- `spec_f114_restore_refuses_a_manifest_of_another_instance`
- `spec_f114_restore_refuses_a_manifest_changed_after_the_preview`
- `spec_f114_restore_moves_the_live_target_aside_then_copies`
- `spec_f114_restore_refuses_an_existing_safety_path`
- `spec_f114_restore_rolls_a_failed_group_back_and_reports_partial`
- `spec_f114_restore_reports_a_failed_rollback_instead_of_claiming_one`
- `spec_f114_a_cancelled_restore_of_a_new_group_reports_cancelled_not_failed`
- `spec_f114_restore_refuses_a_forged_safety_path_outside_the_root`
- `spec_f114_restore_refuses_a_plan_whose_safety_folder_is_outside_the_root`
- `spec_f114_restore_refuses_an_entry_path_that_escapes_the_root`
- `spec_f114_restore_blocks_an_entry_path_outside_its_own_group` (خطة وتنفيذ معًا)
- `spec_f114_pending_restores_reports_a_safety_folder_without_a_manifest`
- `spec_f114_listing_shows_a_legacy_manifest_but_restore_refuses_it`
- `spec_f114_restore_writes_the_lock_last_and_skips_it_when_mods_failed`
- `spec_f114_restore_requires_the_typed_word`
- `spec_f115_backup_omits_saves_by_default`
- `spec_f115_backup_copies_saves_only_when_asked`
- `spec_f115_restore_blocks_saves_entries_by_default`
- `spec_f115_restore_copies_saves_only_when_asked`
- `spec_f118_worlds_preview_writes_nothing` (شجرة قبل/بعد)
- `spec_f118_worlds_preview_lists_a_symlink_without_inspecting_it`
- `spec_f118_worlds_preview_counts_datapacks_and_flags_missing_level_dat`
- `spec_f118_worlds_preview_handles_a_missing_saves_folder`

`src-tauri/src/commands/backup_tests.rs`: تحويل الأكواد (`RestoreMismatch` للهوية، `BackupChanged` للبيان الذي تحرّك، `NewerSchema`)، رفض مجلد غير مسجَّل، تسلسل أحداث القناة مع `terminal`، تحويل `manifestState` إلى DTO، و**الإلغاء على المسار الحقيقي** (`spec_f113_a_cancelled_backup_reports_cancelled_and_copies_nothing`، `spec_f114_a_cancelled_restore_writes_no_content`)، ودورة العلم في `state::tests::spec_f113_a_cancel_reaches_the_flag_the_running_copy_polls`.

## 11. الانحرافات المسجَّلة عن المواصفة (بصراحة)

| # | المواصفة | ما نقصده | لماذا |
|---|---|---|---|
| 1 | F-113: `copytree(symlinks=True)` يحفظ الرابط | **نتخطّى الرابط** ونُسجّله `skipped` بـ`symlink` | التطبيق القديم كان يستطيع إنتاج نسخة **يرفض استعادتها بنفسه** (`_restore_entry` يرفض مجلدًا فيه رابط). نسخة لا تُستعاد ليست نسخة |
| 2 | F-114: `profile.json` مدخل مستعاد | **لا يُستعاد أبدًا**؛ يُنسخ في النسخة كدليل هوية فقط | الصفّ في SQLite هو الحقيقة (`metadata.rs`)، والملف مرآة مشتقّة قابلة لإعادة التوليد. استعادته تُحيي ميتافيزقا تكذب على الصفّ |
| 3 | F-113/F-114: البيان صفوف بمستوى المجموعة (`label`) بلا هاشات | بيان **بمستوى الملف** بهاش ومقاس وإصدار | التطبيق القديم كان يُبلّغ «استُعيدت `mods/`» ولا يعرف أي ملف تلف. مستوى الملف هو ما يجعل «لا حالة نصفية» قابلًا للإثبات. واسم الملف والموضع كما في القديم، فقارئ قديم ما زال يجد `mods/` و`config/` ويستعيدهما |
| 4 | F-113: نسخة بلا بيان تُعرَض وتُستعاد | مجلد بلا بيان = `missing`، **مرفوض للاستعادة** | النسخ المتوقّف يترك مجلدًا؛ اعتباره نسخةً يعني استعادة نصف نسخة بثقة كاملة |
| 5 | F-118: رسائل نصية إنجليزية لكل عالم | `messageCode` + `status` كرموز، والنص من i18n | `AGENTS.md` §5.9: لا نصّ واجهة في النواة، والعربية أول لغة |
| 6 | F-114: الإلغاء بين المدخلات | الإلغاء بين المجموعات **وبين الملفات**، مع تراجع المجموعة | التحسين نفسه الذي يمنع نصف شجرة مستعادة عند إلغاء في المنتصف |
| 7 | F-114: الهدف يُقرأ من مسار البيان | الهدف **من `EntrySource` وحده**، والمسار المسجَّل حكمُ حجب (`entry-outside-group`) | البيان مُدخل غير موثوق: مسارٌ محرَّر لا يجوز أن يوجّه كتابةً خارج النسخة |
| 8 | F-113/F-114: نسخة التطبيق القديم تُستعاد | تُعرض بحالة `legacy` و**تُرفض** للاستعادة | مخططها (`modrinth_bulk_downloader.profile_update_backup.v1`) غير مخططنا، وقراءتها تخمين. الاسم على القرص محفوظ، والعقد ليس |
| 9 | F-113: `copy2` يحفظ `mtime` | **`modified_at` أُزيل من البيان**، والنسخ لا يحفظ وقت التعديل | `std::fs` لا يضبط وقت تعديل بلا تبعية جديدة (`filetime`)، وقاعدة الشريحة «لا تبعية جديدة». حقلٌ يُكتب ولا يُطبَّق يَعِد باستعادة زمنٍ لن يكون هناك؛ والنسخ نفسه (المحتوى) لم يتغيّر |
| 10 | F-114: خمس حالات للمجموعة | **ستّ**، بإضافة `rollback-failed` | مجموعة فشلت وتعذّر إرجاع نسخة أمانها ليست `rolled-back`؛ الحالة الجديدة تسمّيها وترسل المستخدم إلى `safety_path` (§6-4) |

ما عدا ذلك، السلوك مطابق: نسخٌ لا نقل، الوجهة تُزاح لا تُدهس، نسخة أمان ثانوية، كلمة `RESTORE` مكتوبة، والإلغاء افتراضه «لا» في كل موضع.

## 12. البدائل المرفوضة

| البديل | لماذا رُفض |
|---|---|
| **النسخ في مجلد بيانات التطبيق** (`paths::data_dir()/backups/<id>/`) | النسخة تفقد معناها حين تفقد مرجعها: نسخُ مجلد النسخة إلى جهاز آخر أو إعادة تثبيت التطبيق يجعلها أيتامًا بلا صفّ. وداخل النسخة ترافق المحتوى الذي تصفه (الثمن: قد ينسخها المشغّل أو المزامنة السحابية؛ نخفّفه بمجلد مخبأ وبتصدير يستبعدها أصلًا) |
| **الاستعادة بالكتابة فوق الوجهة مباشرة** | فشل في المنتصف يترك شجرة نصفها قديم ونصفها جديد، بلا طريق رجوع — وهو بالضبط ما تمنعه الطبقات الثلاث في F-114 |
| **الاستعادة بنقل ملفات النسخة** (`move` بدل `copy`) | النسخة الواحدة تُستعاد مرة واحدة ثم تختفي؛ والمستخدم يحتاج التجربة والتراجع أكثر من مرة |
| **حفظ كل شيء في أرشيف `.zip`** | يُخفي الشجرة، ويفرض قراءة أرشيف كاملة لسؤال «هل عندي نسخة؟»، ويعيد فتح أسئلة الروابط والحدود التي حلّها `safety::extract`. مسار الأرشيف في هذا المشروع هو تصدير الحزم، لا النسخ |
| **`OperationKind::InstanceRestore` في سجل العمليات** | السجل خاصٌّ بالجهاز ومفكِّك بنوده صارم لكل مود؛ والنسخة تحتاج سجلًّا **يسافر معها** ويُقرأ بعد إعادة التثبيت. البيان يفعل ذلك بلا مخطط جديد |
| **جدول `backups` في SQLite** | المجلد هو الحقيقة: حذف مجلدٍ بيد المستخدم يجب ألّا يترك صفًّا يكذب. الفهرس يُبنى بالقراءة عند الطلب (تكلفة مجلدات قليلة) |

## 13. الأثر

- **لا تبعية جديدة**: `sha2` و`serde_json` و`uuid` و`tempfile` موجودة؛ لا شيء يُضاف.
- **S11 جاهزة للبناء**: التثبيت الآلي يستدعي `build_backup_plan` + `execute_backup` بـ`include_saves: false` قبل أي تحديث، ويستعيد بالخطة نفسها إن فشل — وهذا سبب وجود `BackupSubject` و`now` كبارامترين و`cancel` كعلَم.
- **التصدير لا يتغيّر**: `.modrinth_bulk_downloader_profile_backups` و`..._restore_safety` مذكوران في `EXCLUDED_DIR_NAMES` أصلًا؛ واختبارٌ جديد يُثبت أن مجلد نسخة داخل `config/` (حالة شاذة لكن ممكنة) لا يدخل حزمة.
- **التكلفة**: `list_backups` قراءة بيان لكل مجلد نسخة (بضع عشرات كيلوبايت)، والاستعادة تُهشِّر كل بايت تكتبه (مرور واحد لا مرورين). لا فهرس، ولا مسح خلفية.
- **الحدود المعروفة**: `entries_sha256` ليست توقيعًا (§5)؛ والنسخة لا تحمي من فقدان القرص (هي على القرص نفسه) — النسخ خارج الجهاز ليست في نطاق هذه الشريحة ولا تدّعيها.
- **نافذة انهيار الاستعادة مفتوحة بالتصميم**: لا ذرّية عبر مجموعتين، ولا استعادة تلقائية بعد موتِ العملية بين إزاحة الوجهة ونسخها؛ السطح الوحيد هو `pending_restores` (§6-6) — **قراءةٌ تُعلن، لا إصلاحٌ يدّعي**. ومن يملك الكتابة في مجلد النسخة يستطيع كتابة بيان يوافق ملفاتٍ غير التي نسخناها (§5).
- **نسخ التطبيق القديم مقروءة كأسماء فقط**: تظهر في القائمة بحالة `legacy` ولا تُستعاد (§5). من أراد محتواها فليفكّها بيده؛ لا محوّل في هذه الشريحة.
- **البنود المؤجَّلة تبقى مؤجَّلة**: F-116 (مدير نسخ التحديث لكل ملف) وF-117 (إصلاح الميتافيزقا) وF-119 (استيراد عالم) — لا شيء منها يُنفَّذ هنا، ولا تُبنى له حوامل.
