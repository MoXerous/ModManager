# WS1 brief — behavior, data, compatibility

## قواعد حاسمة — أي خرقٍ يُرفض المخرَج كاملًا
- ممنوع `old_ref` أو `new_ref` يساوي `—` أو `-` أو `unknown` أو فارغًا. كل وحدة يجب أن تشير إلى ملف حقيقي **فتحتَه فعليًا** بTool read/shell في هذه الجلسة، مع سطر أو دالة إن أمكن.
- ممنوع أدلةٌ من نوع `symbol X found in Y` — لا تفتح ملفك الخاص ولا تتحقق من وجود الاسم في مخرَجك. `evidence` يجب أن يذكر ما قرأته فعلًا: اسم الملف + الدالة/السطر + وصف دلالة المقارنة.
- ممنوع تعيين كل الأحكام قيمةً واحدة. إن لم تفتح الملفين فعليًا، اكتب `verdict: "unverifiable"` واشرح السبب — لا تخترع `match`.
- `deviation` يجب أن يكون نصًّا وصفيًا أو `null`؛ وأي `undocumented-deviation` يجب أن يحمل `old_ref` و`new_ref` حقيقيين.
- إذا وجدت نفسك تُنتج قيمًا متكررةً آليةً (نمط `decision=Keep; —`)، توقّف وأعد القراءة من الملفات.

## نطاقك
- 181 وحدة سلوك F-001..F-181 من docs/spec/features.md (جدول المواصفات).
- 12 وحدة بيانات/تنسيقات من docs/spec/data-model.md: الجداول الخمسة، profile.json، instance.lock.json، operation journal، SQLite/WAL، وقراءة ملفات القديم في الجديد.
المجموع: 193 وحدة. لا تضف وحدات أخرى.

## method
لكل وحدة: افتح old_ref في D:\latest_project عند 4f71f18 وnew_ref في E:\ModManager عند HEAD. قارن الدلالة لا الأسماء.
- method=code إن كان الحكم بالقراءة.
- method=test إن كان هناك اختبار يحمل اسم spec_fNNN أو ما يعادله — اذكره في evidence.
- method=run إن شغّلت أمرًا (مثل cargo run -p mm-cli -- ...) — اذكر الأمر ونتيجته.
لا تستخدم browser ولا registry.

## verdicts
match | documented-deviation (إن كان Defer/Drop أو انحرافًا موثّقًا في feature-parity.json) | undocumented-deviation | unverifiable.
severity: blocker | high | medium | low | info. أي undocumented-deviation يجب أن يكون blocker/high/medium.
 reviewer: "deepseek".

## قواعد خاصة
- F-125 جزئي موثّق ⇒ documented-deviation, severity=low, deviation يذكر: للقراءة فقط، بلا كتابة هدف ولا اختيار إصدار آخر.
- لا تفترض أن «الاختبار موجود» يعني «السلوك مطابق» — اقرأ الاختبار.
- إن لم تجد new_ref حقيقيًا، اكتب verdict=unverifiable واشرح في evidence; لا تخترع مسارًا.

## الإخراج
ws1.json واحد، بترتيب A-0001..A-0193، وdimensionCounts {"behavior":181,"data":12}.
