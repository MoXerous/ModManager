# behaviors/jar-metadata.md — بيانات الـjar وكشف الـloader والتعارضات الكاذبة

> **المرجع:** `D:\latest_project\` (تطبيق بايثون/PySide6، `master` @ `4f71f18`) · `app\services\local_mod_metadata.py` · `app\services\instance_detector.py` · `app\services\local_mod_classification.py` · `app\services\content_inventory.py` · `tests\qa\test_content_inventory.py`

---

## 1. قراءة بيانات الـjar — `read_local_mod_metadata(path)`

### 1-1. الثوابت

| الثابت | القيمة |
|---|---|
| `MAX_METADATA_BYTES` | **1 MiB** |
| `MOD_METADATA_FILES` | `{"fabric.mod.json", "quilt.mod.json", "META-INF/mods.toml", "META-INF/neoforge.mods.toml"}` |
| `RESOURCE_TYPES` | `{"mod", "resourcepack", "shaderpack", "datapack", "unknown"}` |

### 1-2. قاعدة الأولوية الصارمة — **أول مطابقة تفوز**

| الترتيب | الملف | اللودر |
|---|---|---|
| 1 | `fabric.mod.json` | `["fabric"]` |
| 2 | `quilt.mod.json` | `["quilt"]` |
| 3 | `META-INF/neoforge.mods.toml` | `["neoforge"]` |
| 4 | `META-INF/mods.toml` | `["forge"]` |

لا مطابقة ⇒ `None`. أي `BadZipFile/OSError/UnicodeDecodeError/JSONDecodeError/ValueError` ⇒ `None` (**بلا رمي ولا تحذير**).

### 1-3. الحقول المقروءة — `LocalModMetadata` (frozen)

`mod_id` · `name` · `version` · `description` · `loaders` · `game_versions` · `source_file`
و`display_name() = name or mod_id`.

| النوع | الحقول | مصدر الإصدارات |
|---|---|---|
| Fabric | `id` · `name` · `version` · `description` | `depends.minecraft` |
| Quilt | `quilt_loader.id` · `quilt_loader.metadata.{name,description}` · `quilt_loader.version` | `quilt_loader.depends` — وهي **قائمة** `{id:"minecraft", versions\|version}` **أو** قاموس `minecraft` |
| Forge/NeoForge (TOML عبر `tomllib`) | `mods[0].{modId, displayName, version, description}` | **تبعية المستوى الأعلى `dependencies`** — أي جدولTECentry بـ`modId == "minecraft"` ⇒ `versionRange` |

### 1-4. التنظيف — `_clean_string`

يقصّ المسافات، **يُفرغ رموز `${…}`**، يطوي المسافات المتتابعة، ويقصّ عند **160 محرفاً**.

### 1-5. تنظيف الإصدارات — `_clean_versions`

يُسقط الفارغ و`"*"`، يزيل التكرار، **ويقصّ عند 8**.

### 1-6. قراءة النص داخل الـzip — `_read_zip_text`

يرفض `info.file_size > MAX_METADATA_BYTES`، يقرأ `MAX + 1` ويرفض التجاوز، ويفكّ بـ`utf-8-sig`.

> **سلوك Forge المؤكَّد باختبار:** `test_reads_forge_jar_metadata_without_new_dependency` — الإصدارات تخرج **كنص `versionRange` الخام** `"[1.20.1,1.21.2)"`.
> ⚠️ **لا يوجد محلّل نطاقات إصدارات.** في Rust قد تختار `node-semver` أو تكتب ~100 سطر — `AGENTS.md` §6 تنصح بتفادي الاعتماد الغامض.

---

## 2. تصنيف نوع المورد — `classify_local_resource_type(path)`

**قبول الاسم:** لاحقة `.jar` أو `.zip`، **أو** اسم ينتهي بـ`.jar.disabled`.
تطبيع الأسماء: `\`→`/`، قصّ `/` البادئة، طيّ `//`.

| الترتيب | الشرط | النتيجة |
|---|---|---|
| 1 | `names ∩ MOD_METADATA_FILES` | `"mod"` |
| 2 | مجلد `shaders` **و** أي اسم ينتهي `.fsh`/`.vsh`/`.glsl`/`.csh` | `"shaderpack"` |
| 3 | يحتوي `pack.mcmeta` **و** مجلد `data` | `"datapack"` |
| 4 | يحتوي `pack.mcmeta` **و** مجلد `assets` | `"resourcepack"` |
| 5 | — | `"unknown"` |

> `pack.mcmeta` يُطابَق **في أي عمق** (`name.endswith("/pack.mcmeta")`) ⇒ `My Pack/pack.mcmeta` يُحسب.
> ⚠️ **لا تمييز Forge/NeoForge في هذه المرحلة** (القاعدة 1 تُرجع `"mod"` دائماً).

**تحذير من نصّ الكود نفسه:** «This data is intentionally informational only. It **must not** make an unknown local file importable as a Modrinth project because it **does not prove** a Modrinth project/version identity.»
> **هذا يطابق `AGENTS.md` §5.12 وقرارنا: لا تُعتمد الميتافيزقا المحلية لإثبات الهوية.**

---

## 3. كشف الـloader — `instance_detector.py`

### 3-1. المصادر — بالترتيب، **الأول يُلغي الباقي**

| # | المصدر | ماذا يُستخرج |
|---|---|---|
| 1 | **`mmc-pack.json`** | مكوّن `uid == "net.minecraft"` ⇒ `version` أو `cachedVersion`؛ ومعرّفات اللودرات |
| 2 | **`modrinth.index.json`** | `dependencies.minecraft` |
| 3 | **`launcher_profiles.json`** (في `minecraft_root` فقط) | `profiles[*].lastVersionId` **مُرشَّحة** بـ`^(\d+(?:\.\d+){1,3}\|\d{2}w\d{2}[a-z])$`، وتُستعمل **فقط إن بقيت قيمة فريدة واحدة** (0 ⇒ تُتجاهل؛ >1 ⇒ إشعار `info`) |
| 4 | **فحص jars** — **بديل فقط** | `sorted(mods_dir.glob("*.jar"))[:30]` |

**`LOADER_UIDS`:** `net.fabricmc.fabric-loader→fabric` · `org.quiltmc.quilt-loader→quilt` · `net.minecraftforge→forge` · `net.neoforged→neoforge`
**`LOADER_FILE_HINTS`:** `fabric.mod.json→fabric` · `quilt.mod.json→quilt` · `META-INF/neoforge.mods.toml→neoforge` · `META-INF/mods.toml→forge`

`BadZipFile` أثناء الفحص ⇒ إشعار `info`.

### 3-2. حارس التعارض الكاذب #1 — **مهم جداً**

`instance_detector.py:264-266` حرفياً:
```
# Launcher/pack metadata is authoritative. JAR inspection is only a
# fallback because NeoForge-compatible archives may also contain the
# legacy Forge metadata path and would otherwise create a false conflict.
```

**التنفيذ:** `if not loaders: self._scan_mod_jars_for_loader(...)`

> **النتيجة:** فحص الـjars **يُتخطّى تماماً** متى عيّن `mmc-pack.json`/`modrinth.index.json` لودراً. فلا يستطيع أرشيف NeoForge-compatible يحمل `META-INF/mods.toml` قديم أن يُنتج `detection_confidence="conflict"`.
> **في Rust: نفس البنية بالضبط — `if loaders.is_empty() { scan_jars() }`.**

### 3-3. حارس التعارض الكاذب #2 — المُصدِّر

`resolver.py:85-89` حرفياً:
```
# Modrinth versions are resolved by game_versions + loaders. Some content
# types do not use normal mod loaders. In particular, shader packs use shader
# platforms such as Iris/OptiFine, and resource packs commonly use "minecraft".
# These conservative fallbacks prevent a saved NeoForge/Fabric profile from
# incorrectly asking Modrinth for a shader version with loader=neoforge.
```

### 3-4. حارس التعارض الكاذب #3 — reasons

`resolver.py:244-249`: رسالة «لم يُعثر» الخاصة بالشيدرات تقول صراحةً إن الباقات «**resolved by shader platform (…), not by Forge/NeoForge directly**».

### 3-5. حارس التعارض الكاذب #4 — مرشّحات الإصدار

`instance_detector.py:356-359`:
```
# Keep this conservative. Launcher profile IDs can include loader IDs
# (for example fabric-loader-...), so only plain Minecraft-style
# versions are used as a likely signal.
```

---

## 4. تصنيف المودات المحلية

### 4-1. الثوابت — `local_mod_classification.py`

```python
CLASS_MANAGED        = "Managed by app"
CLASS_EXTERNAL_UNKNOWN = "External / Unknown"
CLASS_DISABLED       = "Disabled"
CLASS_DUPLICATE      = "Duplicate"
CLASS_INCOMPATIBLE   = "Incompatible"
DISABLED_MODS_DIRNAME = "disabled_mods"
LOCAL_SCAN_SUFFIXES  = {".jar", ".mrpack", ".zip"}
```

- `is_disabled_mod_path` ⇒ `True` إن ended بـ`.jar.disabled` **أو** أي جزء من المسار `casefold`ه إلى `disabled_mods`.
- `iter_local_mod_scan_files(folder)` — **غيرrecursive**: ملفات مباشرة في `folder` + مباشرة في `folder/disabled_mods`؛ إزالة تكرار بـ`str(path)`؛ ترتيب بـ`str(path).casefold()`. مجلد مفقود ⇒ `[]`.

### 4-2. التطابق — أولوية الدليل

`reconcile_content_inventory(local_files, managed_records, *, target_game_version="", target_loader="")`:

1. **SHA-1 أولاً** (`evidence="sha1"`)، ثم تطابق `(provider.casefold(), version_id.strip())` (`evidence="provider_version"`).
2. **حساسية الحالة:** المزوّد غير حسّاس، لكن **معرّفا المشروع والإصدار حسّسان.**
3. **`ownership = "managed"`** إن وُجد مرشّح، وإلا `"external"`.

> **قاعدة ذهبية:** **اسم الملف ومعرّف المشروع وحدهما لا يُثبتان الملكية أبداً** — `test_filename_and_project_id_alone_never_prove_ownership`.
> عند غموض يُفاز **بسجلّ الـmanifest الأول**.

### 4-3. التكرار

مفاتيح `_duplicate_keys` = `sha1:<lowercased>` (إن وُجد) + `provider_project:<provider>:<project_id>` (إن وُجد الاثنان).
يُبلَّغ عن مفتاح **فقط** حين **يعدّاده عبر المجلد الممسوح كله > 1**. كل عنصر يحمل `duplicate_keys` فقط لما تجاوز 1. اللقطة تحمل إشعاراً واحداً لكل مفتاح متجاوز: `duplicate_local_identity`.

### 4-4. تعارض الهاش

```python
hash_mismatch = (evidence == "provider_version"
                 and local.sha1 and record.sha1 and local.sha1 != record.sha1)  # غير حسّاس للحالة
```
⇒ إشعار `managed_hash_mismatch`.

### 4-5. التوافق

| الشرط | الحالة | السبب |
|---|---|---|
| لا هدف إصدار ولا قيد لودر | `unknown` | `"No compatibility target was supplied."` |
| هدف إصدار **و** إصدارات محلية **غير فارغة** **و** الهدف ∉ها | `incompatible` | `"File metadata does not list Minecraft {t}."` |
| قيد لودر **و** لودرات محلية **غير فارغة** **و** القيد ∉ها | `incompatible` | `"File metadata does not list loader {l}."` |
| هدف إصدار **و** قائمة محلية **فارغة** (أو العكس) | `unknown` | `"Compatibility cannot be proven from available metadata."` |
| غير ذلك | `compatible` | `""` |

**القيود المعطَّلة:** لودر فارغ أو `"any"` أو `"minecraft"` **ليست قيداً.**

### 4-6. أنواع الإشعارات — الترتيب

`local` ← `duplicates` ← `managed`:
`external_local` · `ambiguous_managed_match` (شكلان: لكل-محلي ولكل-سجل) · `managed_hash_mismatch` · `duplicate_local_identity` · `missing_local`.

### 4-7. التحقق من المدخلات (`_validate_inputs` — قبل أي عمل)

| الشرط | الخطأ |
|---|---|
| نوع غير `LocalContentFact`/`ManagedContentFact` | `TypeError` |
| `ordinal` أو `size` سالب أو `bool` | `ValueError` |
| `ordinal` مكرر | `ValueError` |
| NUL في أي حقل نصّي | `ValueError` **يسمّي السجلّ والحقل ولا يذكر القيمة** |
| `record_key` فارغ/مكرر/فيه NUL | `ValueError` |
| `game_versions`/`loaders`/`duplicate_keys` ليست `tuple` | `ValueError` |

### 4-8. المحوّل القديم — أولوية التصنيف

`Disabled` → `Duplicate` → `Incompatible` → `Managed` → `External/Unknown`

**نصوص الأسباب الحرفية:**
- `"Disabled marker detected (.jar.disabled or disabled_mods/). No file was changed."`
- `"Duplicate SHA1 detected in the scanned folder: {sha1}."`
- `"Duplicate Modrinth project detected in the scanned folder: {project_id}."`
- `"Matched managed installed_mods.json record by SHA1."` / `"…by version id."`
- `"No managed installed_mods.json record matched by SHA1 or Modrinth project/version identity."`

> **التصنيف قراءة-فقط** — لا إعادة تسمية ولا نقل ولا كتابة في أي مكان (مُثبَت بـ`test_installed_scan_classification_is_read_only_and_profile_aware`).

---

## 5. الاستقلال المتعامد — قرار تصميمي يستحق النقل

`test_managed_disabled_duplicate_incompatible_facts_coexist`:
> «ownership/activation/duplicates/incompatibility are **orthogonal**, not mutually exclusive»

`ContentInventoryEntry` يحمل **أربعة محاور مستقلة**: `ownership` · `activation` · `duplicate_keys` · `compatibility` — ملف واحد يمكن أن يكون **مداراً ومعطّلاً ومكرّراً وغير متوافق في آن واحد.**

> `foundations.md` لا يذكر هذا. **في Rust: `struct ContentFacts { ownership, activation, duplicates, compatibility }` — لا `enum`.** أي `enum` هنا يفقد معلومات ويمنع عرض «معطّل ومكرر معاً».
> كل السجلات `@dataclass(frozen=True)` مع تحويل القوائم إلى `tuple` في `__post_init__`.

---

## 6. الاختبارات

### 6-1. `test_content_inventory.py` (21) — **في الذاكرة بالكامل، بلا قرص**

| الاختبار | الأهمّية |
|---|---|
| `test_records_are_frozen_and_use_tuple_collections` | من-discipline العودية |
| `test_validation_errors_name_the_record_and_field_without_values` | **الرسالة لا تسرّب القيمة** |
| `test_sha1_precedes_provider_version_and_preserves_local_order` | أولوية الدليل |
| **`test_filename_and_project_id_alone_never_prove_ownership`** | قاعدة الملكية |
| `test_version_and_project_ids_remain_case_sensitive` | حساسية الحالة |
| `test_reports_external_missing_ambiguous_and_hash_mismatch_issues` | **ترتيب 8 إشعارات** بالضبط |
| **`test_managed_disabled_duplicate_incompatible_facts_coexist`** | الاستقلال المتعامد |
| `test_compatibility_requires_positive_metadata_effective_evidence` | جدول 7 حالات |
| `test_adapter_delegates_once_and_preserves_legacy_rows_and_reasons` | **نصوص الأسباب الخمسة** |
| `test_canonical_inventory_has_no_io_framework_or_adapter_deps` | AST: 9 استيرادات + 12 نداءً ممنوعاً |
| `test_whitespace_manifest_source_defaults_to_modrinth` | `source.strip() or "modrinth"` |

### 6-2. `test_import_export_resolver_contracts.py`

| الاختبار | ما يثبّته |
|---|---|
| `test_reads_fabric_jar_metadata_without_modrinth_match` | قراءة Fabric |
| `test_reads_forge_jar_metadata_without_new_dependency` | **نطاق Forge الخام** |
| `test_classifies_local_resource_types_from_archive_contents` | الأنواع الخمسة |
| `test_classifies_managed_by_manifest_identity_without_filename_only_match` | بلا مطابقة باسم الملف |
| `test_detects_duplicate_markers_without_mutating_files` | كشف `.jar.disabled` |
| `test_scan_file_iterator_includes_disabled_detection_sources` | القائمة المرتّبة بالضبط `["active.jar","disabled.jar","example.jar.disabled"]` |
| `test_prism_minecraft_child_uses_parent_metadata_and_child_content_paths` | تخطيط Prism |
| `test_metadata_repair_plan_blocks_conflicting_loader_detection` | تعارض `mmc-pack.json` ⇒ `can_apply is False` |

> ⚠️ `test_worker_and_validation_retain_scan_and_io_ownership` و`test_installed_scan_classification_is_read_only_and_profile_aware` — **تأكيدات نص مصدر**.

---

## 7. اقتراح للنقل إلى Rust (غير مُلزِم)

| البند | القرار |
|---|---|
| قراءة الـjar | `zip::ZipArchive` + `read_to_end` بـ`take(1 MiB + 1)` |
| **الأولوية الصارمة** | `const FIRST_WINS: [(&str, Loader); 4]` — جدول واحد بدل سلسلة `if` |
| TOML | `toml` (من `AGENTS.md` §4) — **لا `quick-xml`** |
| JSON | `serde_json` مع `#[serde(default)]` لكل حقل |
| نطاقات الإصدارات | قرار: `node-semver` بعد فحص صفحتها، أو ~100 سطر مكتوب بأنفسنا (`AGENTS.md` §6) |
| التنظيف | `_clean_string`/`_clean_versions` **نفس السلوك حرفياً** — تطبيع `${…}` والقصّ عند 160/8 |
| محاور الجرد | `struct` لا `enum` (§5) |
| كشف اللودر | حافظ على `if loaders.is_empty() { scan_jars() }` **حرفياً** |
| التوافق | أعد `enum Compatibility { Compatible, Incompatible, Unknown }` — القيم النصّية مصدر أخطاء صامتة |

## 8. غير مُتحقَّق

| البند |
|---|---|
| لم أُشغّل أي اختبار |
| ثغرة روابط ZIP الرمزية على ويندوز (`create_system != 3`) |
| سلوك `local_mod_metadata` على jar تالف جزئياً (يقبل `ValueError` بصمت) |
| `instance_detector.py` و`content_inventory.py` تمّ تغطيتهما من ملفَي اختبار decoupled هما |