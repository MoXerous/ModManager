# ModManager

مدير مودات ومود باكات لويندوز ولينكس. **ليس مشغّل ألعاب.**

- تعليمات الوكلاء: [`AGENTS.md`](./AGENTS.md)
- القرارات والأساسيات: [`docs/foundations.md`](./docs/foundations.md)
- مواصفة السلوك المستخرجة من التطبيق القديم: [`docs/spec/`](./docs/spec/)
- قرارات النطاق: [`docs/adr/0003-scope-decisions.md`](./docs/adr/0003-scope-decisions.md)

## بنية المستودع

| المسار | ما هو |
|---|---|
| `core/` | مكتبة Rust بلا اعتماد على Tauri أو الواجهة |
| `cli/` | أداة سطر الأوامر فوق `core` |
| `src-tauri/` | أوامر IPC وأنواع DTO فقط |
| `src/` | تطبيق React |
| `docs/` | الأساسيات · المواصفة · سجل القرارات |

## الأوامر

```bash
pnpm tauri dev                        # تشغيل التطبيق
cargo run -p mm-cli -- info           # أداة CLI
cargo test --workspace                # يولّد أيضاً src/bindings
pnpm build                            # فحص أنواع الواجهة
```

> **مقياس الأداء:** سطر الأوامر الأولية موجود في [`docs/spec/performance-budgets.md`](./docs/spec/performance-budgets.md).