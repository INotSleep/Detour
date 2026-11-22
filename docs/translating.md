# Translating Detour

Detour speaks English, Deutsch, Español, Português (Brasil), Русский, Українська and 简体中文. A language is a set of files of its own in three places; English is the reference, and the build checks that a translation has every text English has.

## Adding a language

Pick its code: the language (`fr`, `ja`), with the region only when it matters (`pt` is Brazilian Portuguese here).

### Desktop app: `src/i18n/<code>/`

1. Copy `src/i18n/en/` to `src/i18n/<code>/`.
2. In each area file, change the top to `import type en from "../en/<area>";` and `const <code>: typeof en = { ... }`, then translate the values. Keys, nesting and the arguments of texts that are functions stay as they are.
3. Counts use `plural(n, { ... })` from `src/i18n/plural.ts`. If your language isn't there, add a function to it that asks for the forms of your language's CLDR plural rules (`one`, `few`, `many`, `other`...; see the [CLDR plural rules](https://www.unicode.org/cldr/charts/latest/supplemental/language_plural_rules.html)), and import it as `plural`.
4. In `index.ts`, set `meta`: the language's own name and its locale for dates and numbers (`{ name: "Français", locale: "fr-FR" }`).
5. Run `npx tsc --noEmit -p .`: it names every text your folder is missing.

The app finds the folder by itself: the language appears in **Settings → Language**, and **System** picks it for people whose system language it is.

### Command line: `crates/detour-cli/src/i18n/<code>.rs`

1. Copy `en.rs` to `<code>.rs` and translate the values. The `help` and `errors` tables keep English's keys on the left.
2. In `mod.rs`, add `mod <code>;` and an entry in `LANGS` with the locale prefix (`("fr", &fr::LANG)`).
3. `cargo test -p detour-cli` checks that the tables have every key.

### iOS: `ios/Detour/Resources/<code>.lproj/`

1. Copy `ru.lproj/Localizable.strings` and `InfoPlist.strings`, and translate the right-hand side (the English text is the key on the left).
2. Add the code to `CFBundleLocalizations` in `ios/project.yml`.

## Style

- Short, as in English. Buttons say what they do.
- No long dashes (— or –): the app's font draws them too narrow. Use a colon, parentheses or a plain hyphen.
- Leave as they are: Detour, SSH, SFTP, product names, commands and their options, file names, key names (Ctrl, Shift).
- Texts in `${...}`, `%@`, `{}` are filled in by the app: keep them, in the order your language needs.
