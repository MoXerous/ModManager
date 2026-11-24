# WS3 brief — packaging M4, dependencies, CLI/IPC

## نطاقك (18 وحدة)
- 8 تغليف: مثبتات Windows (MSI/NSIS) وAppImage/Linux، التوقيع، التحديث التلقائي، ترحيل إعدادات/نسخ القديم، قنوات الإصدار، حجم الحزمة، التشخيص، release manifest.
- 4 تبعيات: crates الأساسية (tokio/reqwest/rusqlite/zip/ts-rs)، حزم npm (React/Tauri/Tailwind)، التراخيص، تاريخ الصيانة.
- 6 CLI/IPC: أوامر mm-cli، أوامر Tauri، IpcError codes، plan-then-execute، dry-run، التقدّم عبر Channels.

## registry إلزامي
لكل تبعية: method=registry وevidence يجب أن يحتوي تاريخ أحدث إصدار والترخيص وحالة الصيانة. استخدم context7 أو صفحة الحزمة عبر webfetch. لا تخمن.

## قاعدة M4
M4 قرارٌ مفتوح في foundations.md وADR 0003 — لا تحكم بـundocumented-deviation لمجرد غياب التغليف؛ صنّفه `documented-deviation` بـ`severity=info` مع الإشارة إلى التعارض الموثّق.

## الإخراج
ws3.json بصيغة {"dimensionCounts":{"packaging":8,"deps":4,"cli":6},"units":[...]}.
