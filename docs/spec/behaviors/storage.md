# behaviors/storage.md — المخطط، والترحيلات، والنسخ الاحتياطي، وبوابة إصدار المخطط

> **المرجع:** `app\storage\schema_contract.py` · `app\storage\migration_runner.py` · `app\storage\migrations\{__init__,v001_contract,v001_baseline,v002_profile_aggregate}.py` · `app\storage\database.py` · `app\storage\repositories.py` · `app\services\operation_journal.py` · `app\operation_models.py`
> **إصدار المخطط الحالي: `2`.**

---

## 1. المخطط الكامل

مصدر الحقيقة: `schema_contract.CREATE_TABLE_STATEMENTS` (الأسطر 4-112)، والعقد column-wise في `CURRENT_COLUMNS` (180-281).

### 1-1. `PRAGMA user_version`

- القيمة الحالية: **`CURRENT_SCHEMA_VERSION = 2`** — `app/storage/migrations/__init__.py:18`.
- القراءة: `migration_runner._read_user_version()` (42-49) → `PRAGMA user_version`، وتُرفع `sqlite3.DatabaseError("user_version returned no row")` إن لم صف، و`("user_version must not be negative")` إن كان سالباً.
- الكتابة: `_set_user_version()` (52-55) تتحقق `isinstance(version, int) and not bool and >= 0` ثم `PRAGMA user_version = {version}` — **سلسلة f مباشرة**، آمنة بحارس `int` وحده.
- تُكتب **داخل معاملة الترحيل**، بعد `_validate_schema` وقبل `commit` (303-308).

### 1-2. الجداول (نسخ حرفي من `CREATE TABLE`)

**`favorites`**
```sql
CREATE TABLE IF NOT EXISTS favorites (
    project_id TEXT PRIMARY KEY,
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT DEFAULT '',
    client_side TEXT DEFAULT 'unknown',
    server_side TEXT DEFAULT 'unknown',
    downloads INTEGER DEFAULT 0,
    follows INTEGER DEFAULT 0,
    project_type TEXT DEFAULT 'mod',
    categories_json TEXT DEFAULT '[]',
    versions_json TEXT DEFAULT '[]',
    date_created TEXT DEFAULT '',
    date_modified TEXT DEFAULT '',
    added_at TEXT DEFAULT CURRENT_TIMESTAMP
)
```

**`profiles`** — 33 عموداً (22 قديمة + 11 من v2)
```sql
CREATE TABLE IF NOT EXISTS profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    game_version TEXT NOT NULL,
    loader TEXT NOT NULL,
    download_dir TEXT NOT NULL,
    notes TEXT DEFAULT '',
    last_download_at TEXT,
    instance_mode TEXT DEFAULT 'classic',
    instance_root TEXT DEFAULT '',
    minecraft_root TEXT DEFAULT '',
    mods_dir TEXT DEFAULT '',
    shaderpacks_dir TEXT DEFAULT '',
    resourcepacks_dir TEXT DEFAULT '',
    saves_dir TEXT DEFAULT '',
    servers_file TEXT DEFAULT '',
    config_dir TEXT DEFAULT '',
    screenshots_dir TEXT DEFAULT '',
    detected_game_version TEXT DEFAULT '',
    detected_loader TEXT DEFAULT '',
    instance_validation_status TEXT DEFAULT 'unknown',
    instance_validation_message TEXT DEFAULT '',
    allow_file_deletion_on_remove INTEGER DEFAULT 0,
    instance_last_validated_at TEXT,
    profile_kind TEXT NOT NULL DEFAULT 'legacy',
    ownership_policy TEXT NOT NULL DEFAULT 'legacy_unknown',
    profile_root TEXT NOT NULL DEFAULT '',
    loader_version TEXT NOT NULL DEFAULT '',
    source_provider TEXT NOT NULL DEFAULT '',
    source_project_id TEXT NOT NULL DEFAULT '',
    source_version_id TEXT NOT NULL DEFAULT '',
    source_manifest_sha256 TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    last_used_at TEXT NOT NULL DEFAULT ''
)
```

**`profile_mods`**
```sql
CREATE TABLE IF NOT EXISTS profile_mods (
    profile_id INTEGER NOT NULL,
    project_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT DEFAULT '',
    client_side TEXT DEFAULT 'unknown',
    server_side TEXT DEFAULT 'unknown',
    downloads INTEGER DEFAULT 0,
    follows INTEGER DEFAULT 0,
    project_type TEXT DEFAULT 'mod',
    categories_json TEXT DEFAULT '[]',
    versions_json TEXT DEFAULT '[]',
    date_created TEXT DEFAULT '',
    date_modified TEXT DEFAULT '',
    preferred_version_id TEXT DEFAULT '',
    preferred_version_number TEXT DEFAULT '',
    preferred_file_name TEXT DEFAULT '',
    preferred_file_sha1 TEXT DEFAULT '',
    preferred_game_versions_json TEXT DEFAULT '[]',
    preferred_loaders_json TEXT DEFAULT '[]',
    added_at TEXT DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (profile_id, project_id),
    FOREIGN KEY(profile_id) REFERENCES profiles(id) ON DELETE CASCADE
)
```

**`download_history`**
```sql
CREATE TABLE IF NOT EXISTS download_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    version_id TEXT,
    version_number TEXT,
    game_version TEXT,
    loader TEXT,
    filename TEXT,
    file_sha1 TEXT,
    destination_path TEXT,
    status TEXT NOT NULL,
    message TEXT DEFAULT '',
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
)
```

**`settings`**
```sql
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)
```

### 1-3. ثغرة `PRIMARY KEY` في SQLite

`favorites.project_id TEXT PRIMARY KEY` و`settings.key TEXT PRIMARY KEY` **لا يعلنان `NOT NULL`**. SQLite تستنتج `NOT NULL` فقط لـ`INTEGER PRIMARY KEY`؛ فـ`NULL` مقبول لمفتاح نصي. `profile_mods` وحدها تُعيد التصريح بـ`NOT NULL` مع مفتاحها المركّب.

### 1-4. الفهارس — **لا شيء صريح**

بحث `CREATE (UNIQUE )?INDEX|CREATE VIEW|CREATE TRIGGER|ATTACH DATABASE|VACUUM|PRAGMA optimize` عبر المشروع كله = **صفر نتائج**. الفهارس الموجودة كلها ضمنية:

| الجدول | الفهرس الضمني | السبب |
|---|---|---|
| `favorites` | `sqlite_autoindex_favorites_1` على `project_id` | PK نصي |
| `profiles` | `sqlite_autoindex_profiles_1` على `name` | `UNIQUE` |
| `profile_mods` | `sqlite_autoindex_profile_mods_1` على `(profile_id, project_id)` | PK مركّب |
| `settings` | `sqlite_autoindex_settings_1` على `key` | PK نصي |

**الأثر العملي:** `list_download_history` (repositories.py:544-589) يفلتر على `status` وينفّذ أربعة تنبيهات `LIKE '%…%'` **بلا أي فهرس داعم** على جدول يكبر بلا حدود.

### 1-5. المفاتيح الأجنبية — **واحدة فقط**

`profile_mods.profile_id → profiles(id) ON DELETE CASCADE`.
مُفعّلة بـ`PRAGMA foreign_keys = ON` على **كل** اتصال (`database.py:23`، `migration_runner.py:243`)، ومُتحقَّق منها بعد الترحيل بـ`PRAGMA foreign_key_check` (`migration_runner.py:197`).

**علاقة واحدة تدمّر بيانات:** `delete_profile` (repositories.py:442-444) تعتمد على `ON DELETE CASCADE` بلا أي سجل تدقيق أو مدخل `download_history` أو سجل عمليات — رغم وجود سجل العمليات تحديداً لجعل هذه التعديلات قابلة للاسترجاع.

### 1-6. الـPragmas المستخدمة (حصر شامل)

| PRAGMA | الموضع |
|---|---|
| `foreign_keys = ON` | `database.py:23`؛ `migration_runner.py:243` |
| `user_version` (قراءة/كتابة) | `migration_runner.py:43` / `:55` |
| `query_only = ON` | `migration_runner.py:145` (اتصال المصدر للنسخة فقط) |
| `quick_check` | `migration_runner.py:148` (اتصال الهدف فقط) |
| `table_info("…")` | `v001_baseline.py:14`، `v002:37`، `migration_runner.py:192` |
| `foreign_key_check` | `migration_runner.py:197` |

> ⚠️ **لا `journal_mode=WAL`، لا `synchronous`، لا `busy_timeout`.** بحث `journal_mode|WAL|synchronous|busy_timeout` عبر `app/` = صفر نتائج. و`docs/superpowers/specs/2026-07-13-database-migration-runner-design.md:371` يصرّح: «No WAL or SQLite tuning change».
> **نمط الحجب الافتراضي `delete` ومهلة الانشغال صفر.** نافذة ثانية أو عمليتان تتعارضان ⇒ `database is locked` فوراً.
> **هذا يناقض `AGENTS.md` §5.10 الذي يفرض WAL — فالتطبيق الجديد يجب أن يفرضه من اليوم الأول.**

### 1-7. سلوك `ClosingConnection`

`class ClosingConnection(sqlite3.Connection)` (10-15): `__exit__` ينفّذ `super().__exit__()` ثم `self.close()` في `finally`. بما أن `__exit__` الأب يُنفّذ `commit`، **كل كتابة في المستودع تُثبَّت بمجرد خروج كتلة `with`** — لا يوجد `conn.commit()` صريح في `repositories.py` إطلاقاً.

### 1-8. المسارات — `app/config.py`

`DB_PATH = DATA_DIR / "app.sqlite3"` (102). ترتيب أولوية الجذور في `_resolve_roots()` (64-97):

1. `MODRINTH_BULK_DOWNLOADER_HOME` مضبوط ⇒ `<override>/{data,config,cache,logs}`
2. `MODRINTH_BULK_DOWNLOADER_PORTABLE` truthy **أو** وجود `portable.flag` بجانب `APP_ROOT` ⇒ `APP_ROOT/user_data/{…}`
3. **توافق قديم:** إن وُجد `~/.modrinth_bulk_downloader/data/app.sqlite3` ولم يوجد مسار المنصّة، ترجع الجذور كلها إلى `~/.modrinth_bulk_downloader`
4. وإلا: تقسيم على `%APPDATA%` / `%LOCALAPPDATA%` (ويندوز)، `~/Library/…` (macOS)، `XDG_*` (لينكس)

**نسخ الترحيل تُكتب بجانب القاعدة داخل `DATA_DIR`.**

---

## 2. الترحيلات

السجلّ: `app/storage/migrations/__init__.py`
```python
CURRENT_SCHEMA_VERSION = 2
MIGRATIONS = (Migration(0, 1, apply_v001), Migration(1, 2, apply_v002))
```

### 2-1. الخطوة `0 → 1` — `apply_v001` (`v001_baseline.py:18-30`)

**إضافية بحتة: نعم.** مرحلتان:

1. تنفيذ `V001_CREATE_TABLE_STATEMENTS` الخمسة بـ`CREATE TABLE IF NOT EXISTS`.
2. لكل `(table, columns)` في `V001_ADDITIVE_COLUMNS`: قراءة `PRAGMA table_info`، تخطي الموجود، وإلا `ALTER TABLE … ADD COLUMN`. التحديث داخل `existing` يجعل إعادة التشغيل **بلا أثر**.

**الأعمدة المضافة لقاعدة قديمة:** `favorites` +10، `profiles` +18، `profile_mods` +16.
**يقرأ أو ينقل ملفات بروفايل؟ لا.** لا وجود لـ`Path`/`os`/`shutil` في الملف — الاستيرادات `sqlite3` + عقد فقط. كلمة «profile» في الاسم تعني **صف قاعدة بيانات** لا مجلداً.

**ثغرة مخطط موثّقة ومقبولة عمداً:** قاعدة `favorites`/`profile_mods` قديمة **لن تكسب `added_at` أبداً** (لا يمكن لـSQLite إضافة `CURRENT_TIMESTAMP` لجدول ممتلئ دون إعادة بناء). موثّقة في `schema_contract.py:284-291` ومرمّزة في `LEGACY_OPTIONAL_COLUMNS`، وتُطرَح من `REQUIRED_COLUMNS` عند 294-297. **هذه هي ثغرة المخطط الوحيدة الموثّقة.**

`v001_contract.py` **نسخة مُجمَّدة** من المخطط بلا أعمدة v2، محروسة بـ`test_v001_contract_does_not_leak_v2_profile_columns`.

### 2-2. الخطوة `1 → 2` — `apply_v002` (`v002_profile_aggregate.py:41-82`)

**إضافية بحتة في DDL: نعم. في البيانات: لا** — وحيدة الترحيلات التي تعدّل صفوفاً موجودة.

1. حارس: `_table_exists(conn, "profiles")` وإلا `sqlite3.DatabaseError("profiles table is required for migration v2")`.
2. إضافة 11 عموداً من `PROFILE_V2_COLUMN_DEFINITIONS` (6-18) بتخطي الموجود.
3. ثلاثة `UPDATE` شرطية:
   - صفوف `instance_mode='instance'` **مع** بقاء `profile_kind='legacy'` **مع** `ownership_policy='legacy_unknown'` ← `linked_instance` / `user_linked`. الحارس المزدوج يمنع الكتابة فوق صف مُصنَّف.
   - `profile_root` فارغ ←=` instance_root` إن وُجد، وإلا `minecraft_root`، وإلا يُبقى.
   - `last_used_at` فارغ و`last_download_at` غير فارغ ← نسخة منه.

**لا يقرأ ملفات البروفايل. SQL بحت.** `created_at`/`updated_at` تُترك `''` عمداً — **لا تواريخ مُلفَّقة**.

### 2-3. فرض التجاور

`validate_migration_registry` (25-61) تُستدعى من `_validated_registry()` (218-226) **قبل لمس ملف القاعدة**:

- `current_schema_version` يجب أن يكون `int >= 0` غير `bool`.
- مشي بسجلّ `expected = 0`، لكل هجرة `from_version == expected` **و** `to_version == expected + 1`؛ وإلا `ValueError("migration registry must be contiguous from version zero")`. هذا يلتقط **الفراغات والتكرار `(0,1),(0,1)` والحوافّ المقلوبة `(1,0)`**.
- بعد الحلقة، `expected != current_schema_version` ⇒ `ValueError("migration registry does not end at current schema version")`.
- **يُحوَّل الخطأ إلى `DatabaseMigrationError("… failed during configuration … The database was not opened by the application.")` قبل `mkdir`** — فسجلّ تالف **لا يستطيع حتى إنشاء ملف القاعدة**. مؤكَّد بـ`test_invalid_registry_fails_before_creating_database_with_safe_context`.

### 2-4. بوابة الإصدار المستقبلي (بوابة `foundations.md` §14)

تُفحص **مرتين**، كلاهما **قبل أي كتابة** (`run_migrations` 245-279):

| # | الموضع | الرسالة |
|---|---|---|
| 1 | قبل القفل، `stage="version-check"` | `DatabaseMigrationError(f"Database migration for {name} failed during version-check: schema version {initial}, which is newer than supported version {CURRENT}. The database was not modified.")` — **بلا نسخة احتياطية، بلا كتابة، بلا معاملة** |
| 2 | بعد `BEGIN IMMEDIATE`، إعادة قراءة | نفس الحارس، لأن عملية أخرى قد ترحّل بين القراءة الأولى وأخذ القفل |

**مُثبَت بايت-ببايت:** `test_future_version_is_rejected_without_backup_or_write` يلتقط `db_path.read_bytes()` قبل وبعد ويطابقهما. ومُثبَت طرف-لطرف عبر `test_self_check_subprocess_rejects_future_database_without_writes` (عملية فرعية، `MODRINTH_BULK_DOWNLOADER_HOME` معزول، رمز خروج 1، بايتات بلا تغيير، لا `*.bak`).

**ومنافس سبقه:** إن كانت `locked_version == CURRENT` بعد القفل، يُلغى ويُعاد `MigrationResult(..., migrated=False)`.

### 2-5. المسارات الثلاث الأخرى

| الحالة | السلوك |
|---|---|
| `user_version == CURRENT` | **مسار سريع**: لا `BEGIN IMMEDIATE`، لا نسخة، لا تحقق. `test_current_database_uses_fast_path_without_backup_or_validation` يتتبّع SQL عبر `set_trace_callback` ويؤكد أن **كل** ما صدر هو حرفياً `["PRAGMA foreign_keys = ON", "PRAGMA user_version"]` |
| `user_version == 0` | `requires_backup = resolved_path.stat().st_size > 0` ⇒ ملف جديد (0 بايت) **بلا نسخة**؛ قاعدة قديمة غير فارغة **بنسخة** |
| `user_version == 1` | تُخطّى `apply_v001`، وتعمل `apply_v002` فقط |

> **حالة مهمّة — قاعدة غير مُصدَّرة لكن بشكلها الحالي:** قاعدة `user_version = 0` وهي بالفعل بشكل v2 (أي أُنشئت قبل كتابة `user_version` أصلاً) تسلك **مسار الترحيل الكامل**: تُعاد كلتا الترحيلتين (idempotent) وتُكتب نسخة احتياطية كاملة. مؤكَّد بـ`test_unversioned_current_database_is_backed_up_once_then_fast` (نسخة واحدة بالضبط، ثم مسار سريع).

---

## 3. أمان الترحيل

### 3-1. آلة المراحل

`preflight` → `open` → `version-check` → `lock` → `backup-check` → `backup` → `migration` → `validation` → `commit` (＋ `close` في `finally`). الكلمة تُستخدم حرفياً في كل رسالة عبر `_safe_error_message` (201-215).

### 3-2. إنشاء النسخة — `_create_backup` (126-175)

**المكان والتسمية** — `_backup_paths` (58-70):
- المجلد: `db_path.parent` (أي `DATA_DIR`).
- الاسم النهائي: `{db}.pre-v{from}-to-v{to}.{YYYYmmddTHHMMSSZ}.{uuid4[:12]}.bak`
- المؤقت: `.{name}.tmp` في المجلد نفسه.
- فحص تصادم قبل البدء ⇒ `FileExistsError("generated migration backup path already exists")`.

**الآلية — واجهة SQLite للنسخ الاحتياطي، لا نسخ ملفات:**
1. `source = connect(db_path)` ثم `PRAGMA query_only = ON`.
2. `target = connect(temp)` ثم `source.backup(target)` — لقطة متّسقة على مستوى الصفحة.
3. تحقق: `PRAGMA quick_check` على الهدف يجب أن يعيد `ok`، وإلا `sqlite3.DatabaseError("migration backup quick_check failed")`.
4. إغلاق الاتصالين في `finally` عبر `_close_backup_connection` (110-123) الذي **يعيد المحاولة مرة** ويعتمد الخطأ؛ ولا يرفعه إلا إن لم يكن هناك استثناء جارٍ — **ففشل الإغلاق لا يخفي السبب الحقيقي ولا يُبتلع صامتاً**.
5. POSIX فقط: `chmod 0o600`.
6. `fsync` على المؤقت.
7. `_publish_backup` — انظر أدناه.
8. `finally` خارجي: إن لم يُنشر، حذف `tmp` best-effort.

**النشر: ذرّي، بلا استبدال، مع write-through.**
- ويندوز: `kernel32.MoveFileExW` عبر `ctypes` بعلامة `MOVEFILE_WRITE_THROUGH` **بلا** `MOVEFILE_REPLACE_EXISTING`. فشل بخطأ `{80, 183}` ⇒ `FileExistsError`، غيره ⇒ `ctypes.WinError`.
- POSIX: `os.link(temp, final)` (رابط صلب — يفشل إذا الوجود موجود، فلا استبدال أيضاً)، ثم حذف `temp`، ثم `_sync_directory` (فتح `O_RDONLY|O_DIRECTORY` ثم `os.fsync`).

> **ملاحظة Windows مهمة:** مسار POSIX بـ`os.link` **لا يُختبر على ويندوز**. الاختباران يرقّيان يرقّيان `_IS_WINDOWS = False`، فلا يُنفَّذ فرع `os.link`/`O_DIRECTORY` الحقيقي في CI على ويندوز.
> **ودورة الوكلاء تثبت أن exFAT لا يدعم الروابط الصلبة أصلاً** (`WinError 1`) — لذا `AGENTS.md` §6 يشترط NTFS لاختبارات نظام الملفات.

### 3-3. توقيت النسخة بالنسبة للقفل

الترتيب: فتح → `foreign_keys=ON` → قراءة `user_version` → **بوابة المستقبل** → `BEGIN IMMEDIATE` → إعادة القراءة → **بوابة المستقبل ثانية** → `requires_backup = st_size > 0` → **النسخة الاحتياطية** → الترحيلات → التحقق → الكتابة → `commit`.

**النسخة تُؤخذ وأنت قفل الكتابة `BEGIN IMMEDIATE` محتجَزاً** (262-289)، عبر **اتصال ثانٍ**. ولأن `BEGIN IMMEDIATE` يحجز قفل RESERVED ولم تكتب شيئاً بعد، يحصل الاتصال الثاني على لقطة نظيفة ومتّسقة.

`test_waiting_runner_rereads_version_and_migrates_only_once` يثبت أن العملية الثانية **تُحجب** عند `BEGIN IMMEDIATE`، وتقرأ `user_version == 2` بعد ذلك، وتعيد `migrated=False` — ومجموع نداءات `apply` **واحد** بالضبط، و`*.bak` واحد.

### 3-4. التراجع عند الفشل

DDL في SQLite معاملي، فـ`ROLLBACK` يُنهي `CREATE TABLE` و`ALTER TABLE`. كلا معالِجَي الاستثناء (315-335):
```python
if conn is not None and conn.in_transaction:
    try: conn.rollback()
    except sqlite3.Error: pass
```
ثم يُعاد رفع `DatabaseMigrationError` حرفياً، أو يُلفّ الأصل في رسالة **منقّاة** مع `raise … from exc` (الرسالة منقّاة، أثر الاستثناء محفوظ).

**فشل الإغلاق لا يحلّ محل الخطأ الأساسي:** `finally` (336-352) يحوّله إلى `DatabaseMigrationError` فقط إذا `sys.exc_info()[0] is None`. مؤكَّد بـ`test_close_failure_does_not_replace_primary_migration_error`.

**النسخة الاحتياطية لا تُتراجَع** — هي ملف على القرص. كل اختبارات الفشل تؤكد بقاء `len(glob("*.bak")) == 1`.

### 3-5. منقّاة رسائل الخطأ

`_safe_error_message` (201-215) يُخرج فقط: **اسم ملف** القاعدة (**لا المسار الكامل**)، كلمة المرحلة، مدى الإصدارين، و`"The application stopped before normal startup."`. الاختبارات تؤكد أن مساراً سريّاً وقيمة صفّ سريّة **لا تتسرّبان**، وأن كلمة المرحلة حاضرة دائماً.

### 3-6. جرد مسارات الفشل (22 مساراً)

| # | المُطلِب | المرحلة | النتيجة |
|---|---|---|---|
| 1 | سجلّ غير صالح | `configuration` | **ملف القاعدة لا يُنشأ** |
| 2 | تعذّر إنشاء المجلد الأب | `preflight` | ملفوف |
| 3 | فشل `sqlite3.connect` | `open` | ملفوف |
| 4 | `user_version` بلا صف / سالب | `version-check` | ملفوف |
| 5 | `user_version > 2` قبل القفل | `version-check` | **بلا كتابة، بلا نسخة** |
| 6 | `user_version == 2` | — | مسار سريع ناجح |
| 7 | `BEGIN IMMEDIATE` محجوب | `lock` | `fail closed`، **بلا نسخة**، والرسالة **لا** تحوي `database is locked` |
| 8 | `user_version > 2` بعد القفل | `version-check` | بلا كتابة |
| 9 | منافس سبقه بالترحيل | — | `rollback` + `migrated=False` |
| 10 | تصادم اسم النسخة | `backup` | المؤقت يُنظَّف |
| 11 | فشل `source.backup` | `backup` | المؤقت يُحذف |
| 12 | `quick_check ≠ ok` | `backup` | المؤقت يُحذف |
| 13 | فشل `close()` لاتصال النسخة | `backup` | يُرفع فقط إن لم يكن هناك خطأ جارٍ؛ المؤقت يُحذف |
| 14 | فشل `chmod`/`fsync` | `backup` | المؤقت يُحذف |
| 15 | الوجود موجود عند النشر | `backup` | POSIX: الرابط نُشر **ويُبقى** |
| 16 | فشل مزامنة المجلد (POSIX) | `backup` | **النسخة المنشورة تُبقى** |
| 17 | `migration.apply` يرفع | `migration` | ROLLBACK؛ النسخة محفوظة |
| 18 | فجوة في السجلّ وقت التنفيذ | `migration` | ملفوف |
| 19 | جدول/عمود ناقص أو `foreign_key_check` | `validation` | **`user_version` يبقى 0** |
| 20 | قيمة `_set_user_version` سيئة | `validation` | ملفوف |
| 21 | فشل `commit()` | `commit` | ROLLBACK **يرجع DDL وكتابة الإصدار** |
| 22 | موت العملية وسط DDL | — | تراجع انهيار SQLite؛ `os._exit(71)` في عملية فرعية ⇒ ماركَر غائب و`user_version == 0` ونسخة سليمة |

---

## 4. سجلّ العمليات — `app/services/operation_journal.py`

### 4-1. المكان والاسم

- الجذر: `DATA_DIR / "operations"` (`composition.py:83,86`).
- الاسم: `{uuid}.json` بمعرّف UUID **أحرف صغيرة قانونية إجبارياً** (`_canonical_operation_id` 351-361 يُحلّل بـ`UUID(value)` ويشترط `value.lower() == str(parsed)`).
- القفل: `.active-operation.lock` — **نقطة، فلا يتصادم مع `*.json`**.
- المؤقتات: `.{operation_id}.{uuid4hex}.tmp` — نقاط، فلا يراها `discover()`.
- حد القراءة لكل سجل: `MAX_OPERATION_JOURNAL_BYTES = 4 MiB` — **على القراءة فقط**.

### 4-2. الشكل

```python
json.dumps(operation_record_to_dict(record), indent=2, sort_keys=True, ensure_ascii=False) + "\n"
```

**18 مفتاحاً علوياً:** `schema` · `operation_id` · `revision` · `kind` · `state` · `created_at` · `updated_at` · `profile_id` · `target_path` · `staging_path` · `backup_path` · 5 عدّادات **مشتقّة لا مخزَّنة** (`planned_count`, `completed_count`, `failed_count`, `cancelled_count`, `skipped_count`) · `message` · `items`.

**كل عنصر 8 مفاتيح:** `item_id` · `project_id` · `title` · `filename` · `state` · `partial_path` · `final_path` · `message`.

**الأنواع:**
- `OperationKind` = `download_batch | update_download | update_install | mrpack_install`
- `OperationState` = `planned | running | completed | failed | cancelled`
- `OperationItemState` = `queued | downloading | verifying | committing | completed | skipped | failed | cancelled`

**التحليل مجموعة مغلقة:** `_reject_unknown_or_missing_keys` (444-454) ترفض **المفاتيح المجهولة والغائرة** معاً على السجلّ والعنصر. و`operation_record_from_dict` **يعيد اشتقاق العدّادات الخمسة ويتحقق منها** (385-397) ويرفع `operation counter mismatch for <key>`. ومعرّفات العناصر المكرّرة مرفوضة في `__post_init__` (165-166).

### 4-3. دلالة `revision`

- `start_operation` يكتب `revision=1`، `state=PLANNED`، و`created_at == updated_at`، مع إعادة توليد UUID في حلقة `while path.exists()`.
- كل طريقة مُغيِّرة تزيد **بواحد** بالضبط: `replace_items`، `transition_operation`، `bind_profile`، `transition_item`.
- **`bind_profile` لا يزيد إن كان المعرّف نفسه** (152-153).
- `transition_operation` إلى الحالة نفسها **لا يزال يزيد ويعيد الكتابة**.
- التزامن المتفائل: `_check_revision` (338-348) — `None` يتخطى، `bool` أو غير `int` يرفع `OperationRevisionConflict("expected operation revision must be an integer.")`.
- القفل `threading.RLock` (43) يُحيط القراءة-التعديل-الكتابة كاملة. مُثبَت بـ`test_concurrent_item_transitions_do_not_lose_updates`: 24 خيطاً، المراجعة النهائية `2 + 24`.

### 4-4. الكتابة الذرّية

`_write_record_unlocked` (306-336):
1. `_ensure_root_for_write()` يرفض جذراً رابطاً رمزياً **(قبل وبعد** `mkdir`)، ويرفض غير-مجلد.
2. يرفض `target.is_symlink()` ⇒ `"operation journal record cannot replace a symlink."`
3. `temp.open("x")` — إنشاء حصري.
4. كتابة + `flush()` + `os.fsync()`.
5. `chmod 0o600` best-effort.
6. **`os.replace(temp, target)`** — ذرّي على NTFS وPOSIX.
7. `_best_effort_fsync_directory` **يعود فوراً على ويندوز**؛ على POSIX يفتح و`fsync` وكل `OSError` مُبتلَع، والـfd يُغلق في `finally`.
8. عند أي استثناء: حذف `tmp` best-effort، ثم `OperationJournalError("could not commit operation journal record: …")` مع إبقاء استثناءات `OperationJournalError` السليمة كما هي (334-335).

### 4-5. الانتقالات المسموحة

**على مستوى العملية:** `planned → {running, failed, cancelled}` · `running → {completed, failed, cancelled}` · النهايات → **مجموعة فارغة**.
**على مستوى العنصر:** `queued → {downloading, skipped, failed, cancelled}` · `downloading → {verifying, failed, cancelled}` · `verifying → {committing, failed, cancelled}` · **`committing → {completed, failed}` — بلا حافة `cancelled`**.

> **قرار تصميمي مقصود:** خطوة الالتزام (commit) **غير قابلة للإلغاء**.

الانتقالات إلى الحالة نفسها مسموحة (عودة مبكّرة في `operation_models.py:280-281`). الحُرّاس: `replace_items` في `PLANNED` فقط · `bind_profile` في `RUNNING` فقط، وإلا `ρ`:"operation is already bound to profile ID N." · `transition_item` بمعرّف مجهول ⇒ `"operation item is missing: <id>"`.

### 4-6. `discover()` — ما يفعله و**ما لا يفعله**

**يفعل:**
- يعيد نتيجة فارغة **بلا إنشاء المجلد** إن لم يكن موجوداً ولم يكن رابطاً رمزياً (212-213).
- يُبلّغ `"operation journal root is a symlink and was not inspected."` ولا يتبعه أبداً.
- يقرأ `sorted(root.glob("*.json"), key=name.casefold())` — ترتيب أبجدي حتمي.
- يتخطّى سجلّاً رابطاً رمزياً بإشعار `"journal record is a symlink and was not read."`.
- **يتحقق أن `path.name == f"{record.operation_id}.json"`**، وإلا `"journal filename does not match operation ID; expected …"`.
- **يلتقط أي استثناء لكل ملف** ويحوّله إلى `OperationDiscoveryIssue`، و`_safe_discovery_error` (371-373) يطوي المسافات ويقصّ عند 500 محرف.

**لا يفعل — كلٌّ منها غياب **مُثبَت** لا افتراض:**

| الغياب | الدليل |
|---|---|
| لا ينشئ الجذر | 212-213 |
| **لا يكتب ولا يسمّي ولا يحذف ولا يعدّل الأذونات ولا يلمس mtime** | لقطة `(bytes, st_mtime_ns)` قبل وبعد متطابقة |
| لا يحذف ولا يجمع السجلّات المنتهية | — |
| **لا يرتّب زمنياً** ولا يفلتر بالوقت | الترتيب أبجدي بالاسم فقط (أي عشوائي لأن الأسماء UUID) |
| لا يتحقق أن `target_path`/`staging_path`/`backup_path`/`partial_path`/`final_path` **موجودة** | — |
| لا يتحقق من هاشات ولا يوفّق الأدلة | `OperationRecoveryAssessment` و`OperationEvidenceStatus` **موجودان كنواع بيانات خالصة ولا يستخدمهما `operation_journal.py` إطلاقاً**؛ و`recovery_action_allowed` مكتوب `return False` (272-274) |
| لا يأخذ قفل `OperationLeaseManager` المجاور له | — |
| لا يُظهر شيئاً للمستخدم بنفسه | دالة صافية |

### 4-7. الإبلاغ عند الإقلاع

آليتان مستقلتان:

1. **العمليات المنقطعة** — `MainWindow.__init__` ينادي `_report_interrupted_operations()` (`main_window.py:4581`)، **بعد** `_build_ui()` و**قبل** `QTimer.singleShot(0, load_remote_metadata)`:
   - لكل إشعار: `"Operation journal warning ({filename}): {message}"`
   - لكل سجلّ ناقص: `"Interrupted operation detected: {kind}; updated={…}; completed={x}/{y}; target={Path(record.target_path).name or 'target unavailable'}. No automatic recovery was attempted."`
   - **يُبلّغ باسم الملف Basename فقط** — إخفاء خصوصية مقصود. الاختبار يؤكد غياب `"Private Name"` و`r"C:\Users"`.
   - الاختبار يؤكد أيضاً أن الدالة **لا تحوي** `ModrinthClient`، `_run_worker`، `unlink(`، `rmtree(`، `shutil.move`، `QMessageBox`.

2. **فشل الترحيل** — `main_window.main()` (14241-14258): `except DatabaseMigrationError` ⇒ `QMessageBox.critical(None, "Database startup failed", f"{exc}\n\nThe application did not modify profile or Minecraft files.")` ⇒ `SystemExit(2)` — **قبل الثيم، والترخيص، وبناء النافذة، والعمال، والشبكة**. خطأ غير متوقّع يُ(propagate) بلا إخفاء.

---

## 5. ملاحظات على تصميم ما يجب ألا ننسخه

>Coverage الكامل في `data-model.md`. الخلاصة هنا:

1. **`get_profile` يُسقط 11 عمود v2 بصمت** (`repositories.py:166-169`)، و`list_profiles` كذلك. أي مستهلك_driving للميتافيزقا لا يعرف أنه لا يملكها.
2. **`ON DELETE CASCADE` صامت** على المحتوى الأساسي.
3. **بلا WAL وبلا busy timeout** ⇒ تعارض فوري بين عمليتين.
4. **`AppRepository.__init__` ينفّذ الترحيلات** عند كل إنشاء — اتصال جديد + قراءة `user_version` **لكل عامل في كل عملية**.
5. **timestamps بثلاثة ترميزات غير متوافقة** في مخطط واحد.

---

## 6. غير مُتحقَّق

| البند |
|---|
| **لم أُشغّل أي اختبار** — كل ما سبق قراءة مصدر، لا تنفيذ |
| فرع POSIX الحقيقي (`os.link` + `O_DIRECTORY`) **لا يُنفَّذ على ويندوز** في الاختبارات |
| `app/services/operation_tracking.py` و`app/ui/workers.py` و`app/application/operation_center.py` لم تُقرأ كاملة — تأكيداتها من ملف الاختبار لا من التنفيذ |
| `docs/superpowers/specs/2026-07-13-database-migration-runner-design.md:371` مقروء عبر grep فقط |
