# Dataloom — mechanics sheet

This sheet records what was learned from `/Applications/TablePlus.app` and the
values used by the recreated desktop app in `app/`. The original was never run.
Only its bundle contents were read.

## Evidence and limitation

| Item | Finding |
| --- | --- |
| Bundle | `/Applications/TablePlus.app` (macOS app bundle) |
| Version | `CFBundleShortVersionString` = `27.2.0`; bundle id `com.tinyapp.TablePlus`; minimum macOS `12.0` |
| Executable | Universal Mach-O (`x86_64` + `arm64`), stripped; `nm -gU` exposes no useful function symbols |
| Interface inventory | 200+ `.nib` files, including `ConnectionWindow`, `DatabaseWindow`, `QueryController`, `QueryResultController`, `TableDataController`, `TableStructureController`, `FilterViewController`, `LeftSidebarController`, `TBTabViewController`, `WorkspaceWindow`, `SQLiteForm` |
| Copy | `en.lproj/Localizable.strings` (1,428 keys) and `Welcome.strings` (menus), plus German, French, Japanese, Russian, Vietnamese and Simplified Chinese localizations |
| Static evidence id | `ev_a2c6125637b61ed2c745be1a68b003be4bda949b0da5b7abe7665e8fbf3f94d1` |

**How deep the pass went.** The arm64 slice was opened with
`core_target_open` (`t_53f2e30e0cbd`) in Deep mode. The overview reports image
base `0x100000000`, 2,197,700 instructions, 50,000 discovered functions, 42,325
strings, 2,438,544 basic blocks and 431 unresolved indirect jumps; discovery
stopped at the 50,000-function limit, so the function list is incomplete.
Ghidra itself is installed, but its macOS decompiler binaries are absent
(`Ghidra/Features/Decompiler/os/mac_arm_64/decompile` is missing), so
`native_function_pseudocode` fell back to the built-in clean-room lifter
(engine `builtin` 0.3.0). Its output is register-level and approximate, so the
function addresses and control-flow branches below are the evidence; the
authoritative exact bytes remain the assembly, the decoded `.nib` objects and
the saved `com.tinyapp.TablePlus` preferences. Values that still could not be
confirmed are marked `estimated` in `mechanics.json` and in the tables below.

### Decompiled functions

These are the functions read with `native_function_pseudocode` for the parts
this build reproduces. Addresses are virtual addresses in the arm64 slice.

| Function | Address | What the pseudo-C shows |
| --- | --- | --- |
| `sub_10071b5e4` | `0x10071b5e4` | `limit <= 0` → "The limitation should be greater than zero"; `limit >= 0x989681` (10,000,001) → "The limitation is too large"; `offset < 0` → "The offset should be positive"; when `offset != 0` and `offset >= total` (`+0x620`) → "The offset should be smaller than the total of rows". On success it stores the limit at the controller's `+0x280` slot and reloads. |
| `-[_TtC9TablePlus15QueryController changeQueryRowsLimit:]` | `0x10038d0c8` | Forwards to `sub_10038cff8` (`0x10038cff8`), which writes the selected limit to `QueryController+0x288` and refreshes the result view. |
| `-[_TtC9TablePlus19TableDataController showPagingPopup:]` | `0x100817948` | Forwards to `sub_100817588` (`0x100817588`), which copies the current limit/offset model values into the popover fields. |
| `-[_TtC9TablePlus13PagingPopover didEndEditing:]` | `0x100532758` | Forwards to `sub_100532a80` (`0x100532a80`), which parses both limit and offset as signed integers (invalid text becomes 0) and sends them to the table controller. |
| `sub_1006f95a0` | `0x1006f95a0` | Quick Filter operator table: Contains, Not contains, Has prefix, Has suffix, each with Case insensitive and Case sensitive variants. It lowercases for insensitive matching, wraps contains with `.*`, uses `$options: "i"` for the Mongo form, and builds `$not` / negated forms for Not contains. |
| `sub_1008168c0` | `0x1008168c0` | Indent-width mapping used by the editor settings: choice 0 → 2 spaces, 1 → 4, 2 → 6, 3 → 8. It then builds the settings dictionary through `sub_1005ab940` and stores it under the `ViewSetting` key. |
| `sub_1005a7988` | `0x1005a7988` | Settings registration: reads each stored settings key and falls back to 0/false when the key is absent. Confirms `DisableAlternatingRows`, `IsDisableRowNumbersInQueryResults`, `HistoryKeepYears`, `LeftSidebarFontSize`, `DefaultDarkThemeID`, `DefaultLightThemeID` and the search-case keys. |
| `sub_1005ab940` | `0x1005ab940` | Rebuilds the settings model from the stored `ViewSetting` dictionary (96 string keys), which is what the settings window and `sub_1008168c0` save. |
| `sub_1006c947c` | `0x1006c947c` | Creates the serial utility queue labelled `com.tableplus.history-save` that writes query history off the main thread. |
| `sub_1005f63c0` | `0x1005f63c0` | Registers the shortcut metadata: Select Next/Previous Pane, Select Next/Previous Tab, Select Next/Previous Workspace, Show Table Data, Show Table Structure, Jump to Column or Row, Toggle Line/Block Comment, Increase/Decrease Font Size, Run Current Query, Run All Queries. |
| `sub_10031c47c` | `0x10031c47c` | Builds the Jump to Column or Row menu and its hint "Use \">\" to search in the column content". |
| `-[_TtC9TablePlus17TableContentTable applySetting]` | `0x1005c8ea0` | Applies table settings through `sub_1005b8474` → `sub_100334074`, which schedules the actual font/padding update on the main queue. |
| `-[_TtC9TablePlus22SettingThemeController changeDataRowPadding:]` | `0x1005a158c` | Theme settings handler; forwards to `sub_1005a1234` with the data-row-padding field offset `+0x2ac`. |
| `-[_TtC9TablePlus22SettingThemeController changeTableDataFontSize:]` | `0x1005a1878` | Forwards with the data-font-size field offset `+0x598`. |
| `-[_TtC9TablePlus22SettingThemeController changeLineHeight:]` | `0x1005a2098` | Forwards with the line-height field offset `+0xcac`. |
| `-[_TtC9TablePlus22SettingThemeController changeEditorFontSize:]` | `0x1005a23ac` | Forwards with the SQL-editor-font-size field offset `+0xa4`. |
| `-[_TtC9TablePlus19TableDataController gotoPage:]` | `0x100817a28` | Dispatches on the page-control segment: previous → `sub_10071b370`, page settings → `sub_10081f074`, next → `sub_10071b144`. |
| `sub_10071b144` | `0x10071b144` | Next page. Loads the stored limit from `QueryController+0x280`, materialises `0x12c` (300), and chooses 300 when the stored limit is lower (`csel …, 0x12c, limit, lt`). The step is therefore `max(limit, 300)`; the offset advances by that step and the table reloads through `sub_10071b7c4`. |
| `sub_10071b370` | `0x10071b370` | Previous page. Uses the same `max(limit, 300)` step and clamps the offset at zero (`bic x8, x8, x8, asr #63`). |
| `sub_10081f074` | `0x10081f074` | Page-settings popover. Shows the same `max(limit, 300)` value in the Limit field and the current offset in the Offset field. |
| `sub_10071b7c4` | `0x10071b7c4` | Reloads the current table content after a paging change; builds the query from the controller model and dispatches the fetch on the main queue. |
| `-[_TtC9TablePlus17TableContentTable changeFontSize]` | `0x1005c8ef0` | Forwards to `sub_1005b84ac` (`0x1005b84ac`), which applies the font change through `sub_100334074` exactly like `applySetting`. |
| `sub_100334074` | `0x100334074` | Shared apply path for table font/padding changes; schedules the actual update on the main queue and passes the table's `+0x90` and `+0x600` fields. |
| `-[_TtC9TablePlus17TableContentTable deleteRow:]` | `0x1005c902c` | Collects the selected rows through `sub_1005c5dc4` (`0x1005c5dc4`), then calls `sub_1005c6fc8` (`0x1005c6fc8`) to delete them. No confirmation dialog is opened. |
| `sub_1005c6fc8` | `0x1005c6fc8` | Deletes the selected rows, registers an undo with `NSUndoManager.registerUndo(withTarget:handler:)`, writes a "deleted rows" status message and refreshes the table. |
| `sub_1005c5dc4` | `0x1005c5dc4` | Builds the list of rows selected for deletion, filtering each row through `sub_1005c5ecc`; rows that fail the predicate are left out. |

### Round 6 pass — functions re-read for this build

The target was opened again with `core_target_open` (`t_53f2e30e0cbd`) and the
functions below were read with `native_function_pseudocode` before this build
was changed. The pseudo-C is register-level and approximate (the macOS arm64
Ghidra decompiler binary is absent), so the addresses, constants and branches
are the evidence. Fresh reads in this pass: `0x10071b5e4`, `0x10071b144`,
`0x10071b370`, `0x10081f074`, `0x100532a80`, `0x1003f6d24`, `0x10038d0c8`,
`0x10038cff8`, `0x100817a28`, `0x100817948`, `0x100817588`, `0x1006f95a0`,
`0x1008168c0`, `0x1005c902c`, `0x1005c5dc4`, `0x1005c6fc8`, `0x10000e300`,
`0x100018d60`, `0x1005a7988`, `0x1005ab940`, `0x1006c947c`, `0x10031c47c` and
`0x1005f63c0`.

| Function | Address | Value / formula / rule |
| --- | --- | --- |
| `sub_10071b5e4` | `0x10071b5e4` | Paging validation. `limit <= 0` → "The limitation should be greater than zero"; unsigned `limit >= 0x989681` (10,000,001) → "The limitation is too large", so 10,000,000 is the largest accepted limit; `offset < 0` → "The offset should be positive"; `offset != 0 && offset >= total` (`+0x620`) → "The offset should be smaller than the total of rows". On success it writes the limit to `+0x280` and the offset to `+0x618`, then calls `sub_10071b7c4`. |
| `sub_10071b144` | `0x10071b144` | Next page. Loads the stored limit from `+0x280`, materialises `0x12c` (300), and uses `csel` to take 300 when the limit is lower. The step is `max(limit, 300)`; it adds the step to the offset and reloads. |
| `sub_10071b370` | `0x10071b370` | Previous page. Same `max(limit, 300)` step; if the offset is already zero it stays zero, otherwise it subtracts the step and clamps the result at zero with `bic x8, x8, x8, asr #63`. |
| `sub_10081f074` | `0x10081f074` | Page-settings popover. Shows the same `max(limit, 300)` value in the Limit field (from `+0x280`) and the current offset (from `+0x618`). |
| `sub_100532a80` | `0x100532a80` | Paging popover handler. Parses the Limit and Offset fields as signed base-10 integers; text that is not a number falls through to 0. It then forwards the two values to the table controller. |
| `sub_1006f95a0` | `0x1006f95a0` | Quick Filter operators: Contains, Not contains, Has prefix, Has suffix, each with Case insensitive and Case sensitive variants (strings `0x10100cca0`–`0x10100cd80`, "Case insensitive" at `0x10100cde0`). Insensitive matching lowercases both sides; Contains wraps the needle with `.*`; the Mongo form appends `$options: "i"`; Not contains builds a `$not`/negated form. |
| `sub_1003f6d24` | `0x1003f6d24` | Connection search refresh. It trims the field with `CharacterSet.whitespaces`, calls `lowercased()`, updates the placeholder "Search for connection..." (`0x101000da0`) and schedules the list refresh on the main queue through `sub_1003ab170` / `sub_1003a9f78`. The recovered rule is trim + lowercase before matching; the field text is not matched case-sensitively. |
| `sub_1008168c0` | `0x1008168c0` | Tab-width mapping: choice 0 → 2 spaces, 1 → 4, 2 → 6, 3 → 8. It then rebuilds the settings through `sub_1005ab940` and stores the dictionary under the `ViewSetting` key. |
| `sub_1005a7988` | `0x1005a7988` | Settings registration. Reads each stored key and falls back to 0/false when absent. Confirms the keys `DisableAlternatingRows`, `IsDisableRowNumbersInQueryResults`, `HistoryKeepYears`, `LeftSidebarFontSize`, `DefaultDarkThemeID`, `DefaultLightThemeID`, `QuerySearchMatchCase`, `QuerySearchMatchWholeWord`, `QuerySearchUseRegularExpression`, `RowSearchMatchCase`, `RowSearchMatchWholeWord` and `RowSearchUseRegularExpression`. |
| `sub_1005ab940` | `0x1005ab940` | Rebuilds the settings model from the 96-key `ViewSetting` dictionary. The saved `com.tinyapp.TablePlus` dictionary supplies `DataFontSize` 11, `DataRowPadding` 12, `LeftSidebarFontSize` 13, `SQLFontSize` 11, `LineHeight` 1.2 (`1.200000047683716`), `TabWidthSpaces` 4, `ThemeID` `builtin:auto`, `HistoryKeepYears` 0 and `QueryTimeOut` 0. |
| `sub_1006c947c` | `0x1006c947c` | Creates the serial utility queue labelled `com.tableplus.history-save` (`0x10100c870`) that writes query history off the main thread. |
| `sub_10000e300` | `0x10000e300` | Row status wording: "Rows (%@ of %@ loaded)" (`0x100ff44a0`) while the total is known, "Rows (%@ so far)" (`0x100ff4480`) when it is not. It also contains "No matching columns or rows" and "Not a valid regular expression". |
| `sub_100018d60` | `0x100018d60` | Truncation and navigation wording: "Only %@ rows loaded" (`0x100ff45f0`), "The table has %@ rows", "Go to row %@ on this page" and "row %@ of the table". |
| `sub_1005c902c` → `sub_1005c6fc8` | `0x1005c902c`, `0x1005c6fc8` | Delete row collects the selected rows through `sub_1005c5dc4`, then deletes them, registers an undo with `NSUndoManager.registerUndo(withTarget:handler:)`, writes a "deleted rows" status and refreshes. No confirmation sheet is opened. |

### Round 7 pass — fresh decompilation for this build

The target was reopened with `core_target_open` (`t_53f2e30e0cbd`) and the
functions below were read with `native_function_pseudocode` before the round-7
source changes. Each function was reached from a string or message with
`native_strings_find`; the returned reference names the function that uses the
string. The pseudo-C is register-level and approximate (the macOS arm64 Ghidra
decompiler binary is absent), so the addresses, constants, comparisons and
control-flow branches are the evidence.

| Function | Address | Value / formula / rule recovered this pass |
| --- | --- | --- |
| `sub_10071b5e4` | `0x10071b5e4` | Paging validation. `limit <= 0` → "The limitation should be greater than zero"; unsigned `limit < 0x989681` (10,000,001) is accepted, so 10,000,000 is the largest limit; `offset < 0` → "The offset should be positive"; `offset == 0` skips the total check; otherwise `offset >= total` (`+0x620`) → "The offset should be smaller than the total of rows". Success stores the limit at `+0x280`, the offset at `+0x618`, then calls `sub_10071b7c4`. |
| `sub_10071b144` | `0x10071b144` | Next page. Loads the stored limit from `+0x280`, materialises `0x12c` (300) and uses `csel x23, x8, x21, lt`; the step is `max(limit, 300)`. It adds the step to the offset and keeps the new offset only when it is below the total, with one boundary branch that can still commit when the controller reports another page. |
| `sub_10071b370` | `0x10071b370` | Previous page. Same `max(limit, 300)` step. An offset `<= 0` is written back as exactly 0; otherwise it subtracts the step and clamps with `bic x8, x8, x8, asr #63`. |
| `sub_10081f074` | `0x10081f074` | Page settings. The Limit field is `max(limit, 300)` from `+0x280`, not the raw stored limit; the Offset field is the stored `+0x618` value. |
| `sub_100532a80` | `0x100532a80` | Paging popover handler. Reads the Limit and Offset fields and parses each as an optional `+`/`-` followed only by decimal digits, with overflow rejected; any other text becomes 0. It does not trim whitespace and does not accept a partial number. |
| `sub_1005a7988` | `0x1005a7988` | Settings registration. Reads the stored keys and falls back to 0/false (or an empty string) when a key is absent. The recovered key names include `DisableAlternatingRows`, `AutoHideTableScollers`, `IsDisableRowNumbersInItemViews`, `IsDisableRowNumbersInQueryResults`, `TableDataFontName`, `QueryEditorFontName`, `LeftSidebarFontName`, `HistoryKeepYears`, `HistoryRetentionPrompted`, `QuerySearchMatchCase`, `QuerySearchMatchWholeWord`, `QuerySearchUseRegularExpression`, `RowSearchMatchCase`, `RowSearchMatchWholeWord`, `RowSearchUseRegularExpression`, `EstimatedRowThreshold`, `LeftSidebarFontSize`, `LeftSidebarPadding`, `DefaultExecuteMode`, `DefaultDarkThemeID`, `DefaultLightThemeID`, `ResultFilterMatchCase`, `DefaultDarkTheme`, `DefaultLightTheme`, `DefaultAvancedFilterOperator`, `DefaultAvancedFilterColumnSort`, `DefaultAvancedFilterColumn`, `DefaultAvancedFilterState` and `DefaultTableOrderBy`. |
| `sub_1008168c0` | `0x1008168c0` | Indent-width mapping: choice 0 → 2 spaces, 1 → 4, 2 → 6, 3 → 8. It then rebuilds the settings model through `sub_1005ab940` and stores the dictionary. |
| `sub_10000e300` | `0x10000e300` | Row-status wording. When the total is known (flag at `+0x18` equals 1) it uses "Rows (%@ of %@ loaded)" (`0x100ff44a0`); otherwise "Rows (%@ so far)" (`0x100ff4480`). The same function owns "No matching columns or rows" (`0x100ff4460`) and "Not a valid regular expression" (`0x100ff4440`). |
| `sub_100018d60` | `0x100018d60` | Truncation and navigation wording: "Only %@ rows loaded" (`0x100ff45f0`), "The table has %@ rows", "Go to row %@ on this page", "row %@ of the table" and "Go to row %@ of the table". |
| `sub_1006f95a0` | `0x1006f95a0` | Quick Filter operator table: Contains, Not contains, Has prefix and Has suffix, each in Case insensitive and Case sensitive form. The case-insensitive path calls `lowercased()` on both sides; Contains wraps the needle with `.*`; the Mongo form appends `$options: "i"`; Not contains builds a `$not`/negated form; Has prefix/Has suffix use `hasPrefix`/`hasSuffix`. |
| `sub_1003f6d24` | `0x1003f6d24` | Connection search refresh. Trims the field with `CharacterSet.whitespaces`, lowercases it, updates the "Search for connection..." (`0x101000da0`) placeholder and refreshes the list on the main queue through `sub_1003ab170` / `sub_1003a9f78`. |
| `sub_10038cff8` | `0x10038cff8` | `changeQueryRowsLimit:` target. Parses the selected menu value and stores it at `QueryController+0x288`, then refreshes the result view. |
| `sub_1005c6fc8` | `0x1005c6fc8` | Delete path. Collects the selected rows through `sub_1005c5dc4`, deletes them, registers an undo with `NSUndoManager.registerUndo(withTarget:handler:)`, writes a "deleted rows" status and refreshes. No confirmation sheet is opened. |
| `sub_1006c947c` | `0x1006c947c` | Creates the serial utility queue labelled `com.tableplus.history-save` that writes query history off the main thread. |
| `sub_1005f63c0` | `0x1005f63c0` | Registers the shortcut identifiers `TPShortcutSelectNextPane`, `TPShortcutSelectPreviousPane`, `TPShortcutSelectNextTab`, `TPShortcutSelectPreviousTab`, `TPShortcutSelectNextWorkspace`, `TPShortcutSelectPreviousWorkspace`, `TPShortcutShowTableData`, `TPShortcutShowTableStructure`, `TPShortcutJumpInGrid`, `TPShortcutToggleLineComment`, `TPShortcutToggleBlockComment`, `TPShortcutIncreaseFontSize`, `TPShortcutDecreaseFontSize`, `TPShortcutRunCurrentQuery` and `TPShortcutRunAllQueries`. |

### Changes applied to the build in round 6


- `src/views.rs`, `paging_window`: non-numeric Limit/Offset text is now parsed
  as 0 and then goes through the four `sub_10071b5e4` checks, matching
  `sub_100532a80`. It no longer rejects the text with a separate message.
- `src/app.rs`, status bar: table tabs now use the `sub_10000e300` wording,
  `Rows (loaded of total loaded)` when the total is known and
  `Rows (loaded so far)` otherwise, followed by the page number derived from
  `max(limit, 300)`.
- `src/db.rs` and `src/theme.rs` keep the paging, tab-width and appearance
  values from the tables above; the code comments name the same addresses.

### Changes applied to the build in round 7

- `src/db.rs`: added `db::parse_paging_text`, which accepts only an optional
  sign and decimal digits and returns 0 for anything else, matching
  `sub_100532a80` (`0x100532a80`). `views::paging_window` now uses it instead
  of `trim().parse()`.
- `src/views.rs`: the page-settings Limit field is seeded with
  `db::page_step(tt.limit)` = `max(limit, 300)`, matching `sub_10081f074`
  (`0x10081f074`).
- `src/views.rs`: the table footer now uses "Rows (N of total loaded)" only
  when the total is known and "Rows (N so far)" when it is not, matching
  `sub_10000e300` (`0x10000e300`). The truncated branch still uses "Only N
  rows loaded" from `sub_100018d60` (`0x100018d60`).
- `src/db.rs`: the SQLite, MySQL and PostgreSQL Quick Filter queries now
  compare `LOWER(CAST(column AS TEXT))` against a lowercased pattern. This is
  the `lowercased()` rule from `sub_1006f95a0` (`0x1006f95a0`) rather than
  relying on the database collation.
- `src/widgets.rs`: the empty grid message is now a parameter; the table grid
  shows "No matching columns or rows" while a filter is active, using the
  string from `sub_10000e300` (`0x10000e300`).
- Tests added: `db::tests::paging_text_matches_sub_100532a80` and an uppercase
  Quick Filter assertion in `runs_queries_browses_tables_and_updates_cells`.

### Recovered evidence


| Address / object | What it shows |
| --- | --- |
| `sub_10071b5e4` (`0x10071b5e4`) | Paging validation: `limit <= 0` → "The limitation should be greater than zero" (`0x10100d2b0`); `limit >= 10,000,001` (assembly materialises `0x989681`) → "The limitation is too large" (`0x10100d290`); `offset < 0` → "The offset should be positive" (`0x10100d270`); `offset != 0 && offset >= total rows` (`+0x620`) → "The offset should be smaller than the total of rows" (`0x10100d230`). On success it stores the limit at `+0x280`, stores the offset and reloads the rows. The limit check is strict, so 10,000,000 is the largest accepted value. |
| `-[_TtC9TablePlus15QueryController changeQueryRowsLimit:]` (`0x10038d0c8`) | Forwards to `sub_10038cff8` (`0x10038cff8`), which stores the chosen limit at `QueryController+0x288` and refreshes. |
| `QueryController.nib` items 367–383 | Row-limit menu titles, with the item **tag** shown in parentheses: "No limit" (no tag, so 0), "100 rows" (1), "500 rows" (2), "1,000 rows" (3), "5,000 rows" (4), "10,000 rows" (5), "50,000 rows" (6), "100,000 rows" (7), "500,000 rows" (8); item 368 is a separator with tag 999. The tags are menu indices, not the numeric limits; the title text carries the numeric limit used by the query. |
| `sub_10000e300` (`0x10000e300`) | Row status strings "Rows (%@ of %@ loaded)" (`0x100ff44a0`) and "Rows (%@ so far)" (`0x100ff4480`). |
| `sub_100018d60` (`0x100018d60`) | Truncation and navigation strings: "Only %@ rows loaded" (`0x100ff45f0`), "Go to row %@", "Go to row %@ on this page", "row %@ of the table". |
| `sub_1006f95a0` (`0x1006f95a0`) | Filter operators: Contains / Not contains / Has prefix / Has suffix, each with Case insensitive and Case sensitive variants (strings `0x10100cca0`–`0x10100cd80`; label "Case insensitive" at `0x10100cde0`). The pseudo-C calls `lowercased()`, `hasPrefix`/`hasSuffix` and `replacingOccurrences`; contains is wrapped with `.*`, the Mongo form appends `$options: "i"` for insensitive matching, and Not contains builds a `$not`/negated form. |
| `sub_1008168c0` (`0x1008168c0`) | Indent width: choice 0 → 2 spaces, 1 → 4, 2 → 6, 3 → 8. The same function rebuilds `ViewSetting` through `sub_1005ab940` and stores the dictionary. |
| Jump to Column or Row | "Use \">\" to search in the column content" (`0x100ffaf50`, used by `sub_10031c47c`) belongs here, not to Quick Filter. Also "Jump to Column or Row" (`0x101008310`), "Column, row number, or value" (`0x100ff4500`), "Search for field" (`0x100ffaea0`). |
| `-[_TtC9TablePlus13PagingPopover didEndEditing:]` (`0x100532758`) | `sub_100532a80` reads a Limit text field and an Offset text field, parses both, and forwards them to the table controller. |
| `-[_TtC9TablePlus19TableDataController showPagingPopup:]` (`0x100817948`) | `sub_100817588` opens the popover with the current limit and offset. |
| `TableDataController.nib` page segments | "Previous page (⌘←)", "Page settings", "Next page (⌘→)". |
| `TableDataController.nib` object 106 (`NSClassSwapper`) | `NSRowHeight = 22.0` and `NSIntercellSpacingHeight = 0.0`. This is the stored table row height, not a value derived from `DataFontSize` + `DataRowPadding`. |
| `sub_10071b144` (`0x10071b144`), `sub_10071b370` (`0x10071b370`), `sub_10081f074` (`0x10081f074`) | Paging step. Each loads the stored limit from `QueryController+0x280`, materialises `0x12c` (300), and selects 300 when the stored limit is lower (`csel …, 0x12c, limit, lt`). The effective step is `max(limit, 300)`, so "No limit" pages by 300 rows. Next page adds the step; previous page subtracts it and clamps at zero. |
| `-[_TtC9TablePlus17TableContentTable deleteRow:]` (`0x1005c902c`) → `sub_1005c6fc8` (`0x1005c6fc8`) | Deleting selected rows goes straight to the undo-registering delete path; no confirmation sheet is shown. The delete path calls `NSUndoManager.registerUndo(withTarget:handler:)`, writes a "deleted rows" status and refreshes the table. |
| `com.tinyapp.TablePlus` preferences | DataFontSize 11, DataRowPadding 12, LeftSidebarFontSize 13, SQLFontSize 11, LineHeight 1.2, TabWidthSpaces 4, DisableAlternatingRows 0, IsDisableRowNumbersInQueryResults 0, ThemeID "builtin:auto" (resolved DarkTheme), HistoryKeepYears 0, QueryTimeOut 0; QuerySearchMatchCase/WholeWord/RegularExpression 0; RowSearchMatchCase/WholeWord/RegularExpression 0. Neither `LimitRowsReturned` nor `LimitQueryRowsReturned` is present. |
| History | `sub_1006c947c` (`0x1006c947c`) creates the serial utility queue `com.tableplus.history-save` (`0x10100c870`) that writes history off the main thread. `0x101028270` is the Swift reflection string `saveHistory(_:connectionModel:)`, not a function address. "Query History for %@" (`0x101018450`) and "Root folder for saved SQL query history for this TablePlus connection" (`0x101018470`) are the UI copy. `HistoryKeepYears` 0 means no automatic trimming. |
| `Base.lproj/Welcome.nib` object 369 | `Run Current Query`, key `\r`, modifier mask `1048576` (Command) → **Cmd+Enter** |
| `Base.lproj/Welcome.nib` object 257 | `Run All Queries`, key `\r`, modifier mask `1179648` (Command+Shift) → **Cmd+Shift+Enter** |
| `Base.lproj/Welcome.nib` object 400 | `New Tab`, key `t`, mask `1048576` → **Cmd+T** |
| `Base.lproj/Welcome.nib` object 261 | `Close Tab`, key `w`, mask `1048576` → **Cmd+W** |

### Round 8 pass — right-hand row detail editor

This pass added TablePlus's vertical row editor on the right of a table's Data
view. The interface inventory came from the original bundle (installed build
27.3.2 / 850 at this pass; full decoded evidence, including nib object indexes,
is in `ROW_DETAIL_EVIDENCE.md`). The row update statement reuses the row-identity
rule already recovered for cell editing (`rowid` on SQLite, integer primary key
on MySQL, `ctid` on PostgreSQL).

| Source | Finding |
| --- | --- |
| `RightSidebarController.nib` | The right sidebar is `RightSidebarController` with outlets `rowLabel`, `rowSearchField`, `rowDetailTable`, `tabView`, `detailsButton`, `assistantButton`. `rowDetailTable` is an `NSTableView` whose cells are `RowDetailTextCell` (`fieldName` + `fieldType` + editable `textField`), `RowDetailByteaCell` (auto-sized `RowDetailTextView`), `RowDetailDropdownCell` (`RowDetailPopupButton`) and chat cells for the Assistant tab. |
| `RightSidebarController.nib` (decoded with the `nibarchive` 1.1.0 parser) | Pane content 300×600. Header 37 high with two 65×19 buttons (`Details` at x=79, `Assistant` at x=154). Search field 254×24 at x=10 (4 pt gap below the header), table 300×542 with a 288-wide column. Prototypes: `RowDetailTextCell` 300×37, `RowDetailDropdownCell` 300×44 (popup 280×24), `RowDetailByteaCell` 300×230 (text view image 276×184, height constraint 184). Field name 175×17 at x=10 and type 110×17 at x=180, 11 pt secondary gray; cell chevron 20×18 with a 14×14 arrow. The split-view row-detail pane has a **240 pt minimum width** (constraint priority 750). |
| `RightSidebarController.nib` strings | Search placeholder `Search for field...`; empty state `No row selected`; matching strategy menu `Matching strategy` ▸ `Fuzzy - default` (checked) / `Contains`; popup converters `EMPTY` / `NULL` / `DEFAULT`; bytea placeholder `NULL`; text-view menu `Auto Pretty JSON`; tab items `Row` / `Assistant` with header buttons `Details` / `Assistant`; actions `filterRows:`, `searchPreferences:`, `updateSearchStrategy:`, `setConstantButton`, `popupButtonDidSelect:`, `enablePrettyJSON:`, `changeTab:`. `copyAction:` / `deleteAction:` / `sendAction:` belong to the Assistant chat cells, not the row detail. |
| `WorkspaceWindow.nib` | The row editor is the workspace's right sidebar template `rightSidebarTemplate`; the `Panels` toolbar `NSSegmentedControl` (65×24) has a segment with tooltip **Toggle Row Detail (Space)** and image `rightSidebarTemplate`, action `toggleSplitView:`. Related runtime names: `toggleRightSidebar:`, `rightSidebarController`, `rowDetail`, `rowDetailTable`. The nib stores no Space key equivalent; the tooltip is the evidence for the shortcut. |
| TablePlus 27.3.2 toolbar Commit control | The original stages edits and commits explicitly: a 3-segment toolbar control with `Discard all the changes (Cmd+Shift+Delete)`, `Preview your changes as SQL Statements (Cmd+Shift+P)` and `Commit your changes (Cmd+S)`; localizable text confirms "You can commit the changes by 1. Command + S. 2. Use the top left segment control." This build deliberately auto-saves instead (see below). |
| `en.lproj/Localizable.strings` | `Search for field`, `Details`, `All fields...`, `Row detail text`, `Set Value                ⌥↩`, and the NULL→EMPTY conversion wording. |
| Field type labels (provided reference screenshot) | TablePlus prints `long` for integer columns, `var_string` for character columns and `blob` for text/blob columns in the detail cells. |
| Row identity / update | No separate update function was recovered for the sidebar; the build reuses the same row identity as in-cell editing and sends `SET <column> = ?` with a bound parameter. `NULL` is sent as SQL NULL and `EMPTY` as an empty string. |
| Type labels | TablePlus computes them from the driver's native type names: MySQL `LONG` → `long`, `VAR_STRING` → `var_string`, `BLOB` → `blob` (confirmed by the `MySQL97.framework` enum-name block and the reference screenshot); no literal `var_string` appears in the app binary or nibs. |

Implementation notes that are recreated, not recovered:

- The panel is resizable (like the original's `NSSplitView` right sidebar),
  defaults to 336 pt and never shrinks below the nib's 240 pt minimum; the
  300 pt cell width is the reference for the content column.
- The value editors auto-size multi-line text to the wrapped galley, matching
  `RowDetailTextView`; fields with one short value use a single line so Enter
  saves, while long text/blob values use Enter for a newline and Cmd+Enter to
  save. TablePlus instead stages every field into a pending transaction and
  commits with the toolbar control / Cmd+S; this build auto-saves a field when
  it loses focus or on Enter. That is a deliberate deviation recorded in
  "Recreation choices", not a claim about the original.
- The chevron menu on each value offers `Set NULL`, `Set empty` and
  `Copy value`. The original's menu strings show `NULL`, `EMPTY` and `DEFAULT`
  conversions; DEFAULT is not implemented because this build has no per-column
  default sentinel.
- The empty state is the nib's `No row selected`. Views and keyless tables show
  the fields read-only because there is no row identity to update.
- Leaving a tab (sidebar table, tab bar, Cmd+T), closing it, disconnecting or
  switching connections flushes a focused field's typed value before the tab is
  torn down. MySQL reports 0 changed rows for a same-value update, so that case
  is reported as "Saved … (no change)"; SQLite/PostgreSQL count a matched row,
  so 0 there means the record disappeared and is reported as an error. After a
  save the row is reselected by key (rowid / integer primary key); PostgreSQL's
  `ctid` changes on UPDATE, so that backend falls back to the old row index.
- The row detail editor is implemented for table Data tabs. TablePlus also
  shows it for query results; that path is not wired yet.

### Round 9 pass — table context menu and table operations

The sidebar table menu follows TablePlus's `LeftSidebarController.nib` menu. Only
the five operations requested for Dataloom were kept.

| Source | Finding |
| --- | --- |
| `LeftSidebarController.nib` menu | Table items: `Open in new tab`, `Open structure`, `Item overview`, `Show Diagram`, `Copy name`, `Pin to top`, `Export`, `Import`, `New`, `Copy Script As`, `Clone...`, `Truncate...`, `Delete...`, `Generate diagram`. Actions: `cloneTable:`, `truncateTable:`, `exportTable:`, `exportTableWithColumnSelection:`, `importSQL:`, `importSQLFile:`, `copyTruncate:`, `__importCSVFile:`, `__importJSONFile:`. |
| `en.lproj/Localizable.strings` | Export destinations the original supports include `Export table '%@'`, `Export data as EXCEL`, `Export CSV`, `Export data as JSON`; menu/dialog copy includes `Clone...`, `Truncate...`, `Delete...`, `Delete “%@”?`, `Import`, `Import...`. |
| Dataloom subset | The recreation keeps exactly **Export (SQL / CSV / XLSX)**, **Import (SQL)**, **Clone**, **Truncate**, **Delete**. Format-specific items are a submenu; the other original entries (open structure, diagram, copy name, pin, new, copy script) are not reproduced. |
| XLSX writer | Dataloom has no spreadsheet dependency: `src/xlsx.rs` writes a minimal OOXML workbook in a hand-rolled STORE-only ZIP (five XML parts, CRC-32, central directory). Single sheet, inline strings, one header row; no styles, formulas or dates-as-dates. Validated by loading the output with `openpyxl`. |
| CSV writer | UTF-8 with a BOM (so Excel detects the encoding), CRLF rows, RFC 4180 quoting; SQL NULL and empty string both become an empty field. |
| SQLite clone | `CREATE TABLE` text is copied from `sqlite_master` with only the table name rewritten (quoted, `IF NOT EXISTS`, bracket/backtick forms), then `INSERT INTO new SELECT * FROM old`; primary keys and constraints survive, secondary indexes are not copied. MySQL uses `CREATE TABLE ... LIKE`, PostgreSQL uses `LIKE ... INCLUDING ALL`. |
| Truncate / delete | SQLite has no `TRUNCATE`, so Dataloom runs `DELETE FROM` and resets `sqlite_sequence` when it exists; MySQL/PostgreSQL use `TRUNCATE TABLE`. Delete uses `DROP TABLE`, closes the matching tab and refreshes the schema. Clone, truncate and delete use true `egui::Modal` dialogs (dimmed backdrop, clicks blocked outside, Esc/backdrop cancels); clone has an editable name. |

Known limitations of this pass, not claims about the original:

- Clone is not transactional. SQLite preserves the `CREATE TABLE` text (keys and
  constraints) but not secondary indexes; MySQL `CREATE TABLE ... LIKE` and
  PostgreSQL `LIKE ... INCLUDING ALL` copy indexes, but `INSERT ... SELECT *`
  can fail on generated columns, and PostgreSQL identity columns may need
  `OVERRIDING SYSTEM VALUE`.
- Exports read every row through the existing table fetch, so binary columns
  that the grid renders as `"<BLOB n bytes>"` are exported as that placeholder;
  MySQL binary/non-UTF-8 values are lossy the same way the data grid is.
- CSV is a data export, not a spreadsheet-safe one: formula-looking strings are
  not prefixed, and NULL and empty string both become an empty field.
- The XLSX writer is deliberately minimal: one sheet, inline strings, no styles,
  column widths, formulas or true date cells.

### Round 10 pass — connection groups and connection menu

The connections screen now groups saved connections into flat folders and moves
the per-connection actions into a right-click menu, following the original's
connection picker.

| Source | Finding |
| --- | --- |
| `ConnectionWindow.nib` | Menu items `Connection...`, `Edit...`, `Duplicate`, `Delete`, `Export Connections`, `Import Connection`; actions `newConnection:`, `editConnection:`, `cloneConnection:`, `deleteConnection:`, `exportConnection:`, `importConnection:`, `copyConnectionURL:`. Outline classes `ConnectionOutline` and `ConnectionOutlineCell`. |
| `CreateConnectionWindow.nib` | `New Group` button; `importConnectionFrom:` action. |
| `LeftSidebarController.nib` | Group menu copy: `New Group` / `New Group...`, `Edit group...`, `Edit/Rename...`, `Delete group`. |
| `en.lproj/Localizable.strings` | `Duplicate`, `New Group`, `New Group…`, `Import Connection`, and copy about duplicating a connection to This Mac. |

Implementation notes that are recreated, not recovered:

- Groups are **flat** folders stored in `connections.json` as
  `"groups": [{"id":…,"name":…}]`; each connection has an optional `group_id`.
  Old store files without those keys still load (serde defaults), and a
  connection whose group disappeared is shown as ungrouped.
- The connections screen renders an `UNGROUPED` section (when there are both
  groups and ungrouped connections) followed by one section per group, with a
  folder icon and connection count. Group headers have their
  own right-click menu: `Rename group…` and `Delete group…`, both opening a
  modal; deleting a group keeps its connections and moves them back to
  ungrouped.
- Connection cards keep only **Connect**; the right-click menu offers
  **Edit…**, **Duplicate**, **Export…** and **Delete…**. The delete action still
  uses the existing confirmation modal. Duplicates keep the same settings and
  group, and get a unique "copy" name.
- Connections are imported from JSON: a single object, an array,
  `{ "connections": [...] }` or a Dataloom `{ "connection": … }` export. Imported
  ids/groups are local-only and are dropped; names are made unique when needed.
- Connection export writes one JSON object with the connection settings and an
  empty password. Passwords are not exported; ids and group ids are cleared so
  the file can be imported on another machine.
- Import/export use the native macOS file dialogs, like the SQL tools.

### Open questions the bundle could not settle

| Question | Severity | What was done instead |
| --- | --- | --- |
| Default query row limit | resolved | No limit (0). Neither `LimitQueryRowsReturned` nor `LimitRowsReturned` is read by the executable, and the first menu entry, "No limit", has no tag. |
| Default table limit | resolved | 300 rows. `sub_10071b144`, `sub_10071b370` and `sub_10081f074` all fall back to `0x12c` (300) when the stored limit is lower, including "No limit" (0). |
| Exact row height | resolved | `TableDataController.nib` object 106 stores `NSRowHeight = 22.0`; `NSIntercellSpacingHeight = 0.0`. The earlier 23 pt value came from adding the font size and row padding and was not the nib's row height. |
| Exact key equivalent for "Run Current Query" | resolved | `Welcome.nib` object 369 confirms Cmd+Enter |
| Exact palette values of the default TablePlus theme | low | Built a neutral dark/light palette; not sampled from the asset catalog |
| Server-side behaviour for MySQL/PostgreSQL/SSH/admin tools | medium/high for those features | SQLite, MySQL and PostgreSQL connections, queries and table editing are implemented; SSH tunnelling, safe mode, server administration and the AI/MCP features are not |

These gaps are limits of the built-in lifter (no macOS arm64 Ghidra decompiler
binary), not assumptions that the values are correct. The decompiled functions
above still settle the paging, row-limit, tab-width, filter and default-setting
rules used by this build.

## What the original is

TablePlus is a desktop database client. The bundle's forms and menus show the
core loop:

1. Manage saved database connections, groups and SSH settings.
2. Open a workspace for one connection.
3. Browse schemas, tables and views in a left sidebar.
4. Run SQL in a tabbed editor and inspect rows in a result grid.
5. Browse a table's data and structure, with filtering, paging, editing and export.
6. Keep several query and table tabs open in one workspace.

The recreated app keeps that loop for SQLite, and adds SQL export/import and
backups in a simpler form than the wizards. Network drivers, SSH, metrics
dashboards, AI chat and the MCP server are outside this version.

## Mechanics used by Dataloom

| Mechanic | Value | Group | Source |
| --- | --- | --- | --- |
| Driver | SQLite | connection | `SQLiteForm.nib` ("File path", "Select file..."); separate MySQL/PostgreSQL/... forms exist |
| Connection fields | name + database file path | connection | `SQLiteForm.nib` |
| Start state | the connections screen on every launch; the previous connection is not reopened | connection | Dataloom choice — the original reopens the last saved connection, otherwise the first saved one (`ConnectionPicker.nib`, `Welcome.strings`) |
| Sidebar tree | tables and views with row counts | browsing | `LeftSidebarController.nib`; menu items "Show Table Data", "Show Table Structure" |
| Sidebar search | filter tree by name | browsing | `Localizable.strings`: "Search for connection...", "Find Items in left sidebar..." |
| Query editor | SQL, multiple statements, current-query run | query | `QueryController.nib`; "Run Current Query", "Run All Queries", "Stop" |
| Run shortcut | Cmd+Enter | shortcuts | `Base.lproj/Welcome.nib` menu item 369: key `\r`, mask 1048576 (Command) |
| Run all shortcut | Cmd+Shift+Enter | shortcuts | `Base.lproj/Welcome.nib` menu item 257: key `\r`, mask 1179648 |
| New tab shortcut | Cmd+T | shortcuts | `Base.lproj/Welcome.nib` menu item 400 |
| Close tab shortcut | Cmd+W | shortcuts | `Base.lproj/Welcome.nib` menu item 261 |
| Table page shortcuts | Cmd+Left / Cmd+Right | shortcuts | `TableDataController.nib` page-control tooltips |
| Statement splitting | semicolons outside quotes/comments | query | inferred from "Run All Queries" — **estimated** |
| Query row limits | No limit, 100, 500, 1,000, 5,000, 10,000, 50,000, 100,000, 500,000 | results | `QueryController.nib` items 367–383 (menu tags 1–8; titles are the values); `changeQueryRowsLimit:` `0x10038d0c8` → `sub_10038cff8` `0x10038cff8` stores the chosen limit at `+0x288` |
| Default query row limit | No limit (0) | results | `LimitQueryRowsReturned`/`LimitRowsReturned` are not referenced by the executable; the first menu entry "No limit" has no tag (0) |
| Result grid | row-number gutter, column headers, alternating rows | results | "Alternating row background colors", "Show line numbers in query results", "Rows (%@ of %@ loaded)" |
| Row status | "Rows (loaded of total loaded)" while partial; "Only N rows loaded" when the page is capped | results | `sub_10000e300` (`0x10000e300`); `sub_100018d60` (`0x100018d60`) |
| Quick Filter | case-insensitive contains across columns | browsing | Quick Filter field; full operator set and regex construction from `sub_1006f95a0` (`0x1006f95a0`) |
| Quick Filter operators | Contains, Not contains, Has prefix, Has suffix; case-insensitive or case-sensitive | browsing | `sub_1006f95a0` strings `0x10100cca0`–`0x10100cd80`, "Case insensitive" at `0x10100cde0` |
| Column filters | per-column filters (column + operator + value), combined with ALL (AND) or ANY (OR) on top of the Quick Filter | browsing | Dataloom choice — the original stores the advanced-filter state in the `DefaultAvancedFilter*` `ViewSetting` keys; operators mirror `sub_1006f95a0` |
| Jump to Column or Row | `>` searches in the column content | browsing | "Use \">\" to search in the column content" (`0x100ffaf50`); "Jump to Column or Row" (`0x101008310`) |
| Default table limit | 300 rows | browsing | `sub_10071b144` (`0x10071b144`), `sub_10071b370` (`0x10071b370`), `sub_10081f074` (`0x10081f074`): `csel …, 0x12c, limit, lt` falls back to 300 |
| Page step | `max(limit, 300)` rows | browsing | same three paging functions; next page adds the step, previous subtracts it and clamps at zero |
| Pagination | Limit/Offset page-settings popover, previous/next page segments | browsing | `PagingPopover didEndEditing:` (`0x100532758`), `sub_100532a80`, `showPagingPopup:` (`0x100817948`), `gotoPage:` (`0x100817a28`), `TableDataController.nib` page segments |
| Paging validation | 1 ≤ limit ≤ 10,000,000; offset ≥ 0; offset = 0 or offset < total rows | browsing | `sub_10071b5e4` (`0x10071b5e4`); limit compared against `0x989681` (10,000,001) |
| Delete row | confirmation, then delete; the original deletes immediately with undo | editing | `TableContentTable deleteRow:` (`0x1005c902c`) → `sub_1005c6fc8` (`0x1005c6fc8`): `NSUndoManager.registerUndo`, "deleted rows" status, table refresh; no confirmation sheet. Dataloom keeps a confirmation step and has no undo history yet. |
| Table editing | double-click a cell and edit it in place; Enter saves, Escape cancels, clicking away saves | editing | Dataloom choice — the grid is custom-painted; the original edits cells inside `TableContentTable` ("Add Row", "Commit", "Discard", "Paste to Selected Cells") |
| Export SQL | one table or the whole database: `CREATE` plus one `INSERT` per row, written to a chosen path | tools | Dataloom choice — the original stores `ExportTableSQLNotIncludeStruct`, `ExportTableSQLNotIncludeContent`, `ExportTableSQLDropIfExist`, `ExportTableSQLCompressGzip` and `ExportFileNamePatterns` in `ViewSetting` |
| Import SQL | runs every statement of a `.sql` file against the connection | tools | Dataloom choice — the original offers an "Import SQL Wizard" and "From SQL Dump..." menu items |
| Backups | timestamped backup per connection in the app data folder: byte-for-byte file copy for SQLite, SQL dump for the server engines; SQLite copies can be restored in place | tools | Dataloom choice — the original has `BackupWindow`, `BackupRunner`, `BackupHistoryWindow`, `BackupFileNamePatterns` and a "Backup database (Legacy)..." menu item |
| Structure view | columns, types, nullability, defaults, primary keys, indexes, DDL | structure | `TableStructureController.nib`; "Show table structure", "Columns (%@)" |
| Tabs | many query/table tabs, closable | tabs | `TBTabViewController.nib`; "New Tab", "Close Tab", "Select Next Tab" |
| Query history | written to disk; per connection in the original, one global file in Dataloom | query | `saveHistory(_:connectionModel:)` (`0x101028270`), `com.tableplus.history-save` (`0x10100c870`), "Query History for %@" (`0x101018450`) |
| History retention | keep all (`HistoryKeepYears` 0, no automatic trimming) | query | `ViewSetting` → `HistoryKeepYears` = 0; read by `sub_1005a7988` |
| Themes | light and dark palettes; Dataloom opens light on every launch, the toolbar toggle switches to dark | appearance | `SettingThemeController.nib`; the original follows the system through `ViewSetting` → `ThemeID` = "builtin:auto", Dataloom pins the light palette (`theme::DEFAULT_DARK` = false) |
| Query timeout | 0 (no timeout) | query | `ViewSetting` → `QueryTimeOut` = 0; the MCP tool schema separately clamps `time_out` to 1–300 s (default 300) |
| Window size | 1400 × 880 pt, minimum 980 × 620 | appearance | recreation choice — **estimated** |
| Sidebar width | 297 pt | appearance | `NSSplitView Subview Frames WorkspaceSplitView` saved 296.5; 297 is the rounded value |
| Grid row height | 22 pt | appearance | `TableDataController.nib` object 106 (`NSClassSwapper`): `NSRowHeight = 22.0`, `NSIntercellSpacingHeight = 0.0`; table apply path `TableContentTable applySetting` `0x1005c8ea0` → `sub_1005b8474` → `sub_100334074` |
| Data font size | 11 pt | appearance | `ViewSetting` → `DataFontSize` = 11 |
| Sidebar font size | 13 pt | appearance | `ViewSetting` → `LeftSidebarFontSize` = 13; read by `sub_1005a7988` (`0x1005a7988`) |
| SQL font size | 11 pt | appearance | `ViewSetting` → `SQLFontSize` = 11 |
| Line height | 1.2 | appearance | `ViewSetting` → `LineHeight` = 1.2 (stored as the string "1.200000047683716"); editor line height = 11 × 1.2 = 13.2 |
| Tab width | 4 spaces | appearance | `ViewSetting` → `TabWidthSpaces` = 4 |
| Tab width choices | 2, 4, 6, 8 spaces | appearance | `sub_1008168c0` (`0x1008168c0`) maps choice 0–3 to 2/4/6/8 |
| Alternating rows | on by default | appearance | `ViewSetting` → `DisableAlternatingRows` = 0; read by `sub_1005a7988` |
| Row numbers | on by default | appearance | `ViewSetting` → `IsDisableRowNumbersInQueryResults` = 0; read by `sub_1005a7988` |
| Accent color | `#4F8CFF` | appearance | neutral recreation palette, not sampled from the original — **estimated** |

## Recreation choices that are not claims about the original

- **Neutral identity.** The app is called Dataloom and uses a four-square mark
  drawn from shapes. TablePlus logos, icons and long copy were not copied.
- **Drivers.** SQLite, MySQL and PostgreSQL connections are implemented; SSH
  tunnelling, safe mode, server administration, AI chat and the MCP server are
  not.
- **Sample data.** A fresh install creates a small shop database so the app is
  usable immediately. The tables (`customers`, `products`, `orders`,
  `order_items`, `employees`) and view (`active_products`) are invented. The
  sample connection is seeded on the very first launch only: once
  `connections.json` exists, an empty store is respected, so deleting the sample
  connection (or every connection) is not undone on the next launch.
- **Column filters.** The filter bar adds per-column filters (column, operator,
  value) on top of the Quick Filter, combined with AND by default or OR through
  "match any". Values are bound or escaped, never interpolated; LIKE wildcards in
  a value are matched literally; unknown columns and blank values do not
  constrain the query.
- **Connections screen on launch.** Every launch starts on the saved-connection
  list (`live = None`); no connection or tab state is restored, so connecting is
  an explicit click. The original reopens the last saved connection.
- **Asynchronous queries and cancellation.** The SQL editor submits work to a
  dedicated worker thread with its own connection, so a slow statement does not
  freeze the window. The tab shows a spinner and elapsed time; `Cancel` asks the
  driver to stop: SQLite uses its interrupt handle, PostgreSQL its cancel token,
  and MySQL a separate connection running `KILL QUERY`. A per-request flag skips
  a query that was cancelled before it entered the blocking call. Results are
  routed by a stable tab id, so switching tabs while a query runs is safe, and a
  closed tab cancels its query. Session state (temporary tables, variables)
  lives on the worker connection; table browsing and cell edits still use the UI
  connection, and table paging remains synchronous in this pass. The toolbar
  also swaps Run for Cancel while a query is in flight. There is no configurable
  query timeout yet.
- **Header sorting.** Clicking a column header sorts the table server-side
  (`ORDER BY` built from the validated column list) and cycles
  ascending → descending → unsorted; Shift-click adds or toggles extra columns
  for a multi-column sort. Query results sort in memory and keep the original
  order so clearing the sort restores it. A plain click also resets the table to
  the first page so the new order is visible. Unsorted headers always show a
  faint up/down glyph, hovered headers get a background, the active column gets
  an accent triangle (plus its position number in multi-sort), and the header
  context menu offers explicit Sort ascending / Sort descending / Clear sort
  next to the width actions. The table footer reads "Click a column header to
  sort" or "Sorted by … (asc|desc)" with a Clear sort button.
- **Column widths.** The data grid measures each column with the real font and
  the configured data font size, then lets the user drag the separator between
  headers to resize (48–900 px). Double-clicking a separator auto-fits that
  column; the header's context menu offers Auto-fit column, Auto-fit all and
  Reset widths. Widths live on the tab (`TableTab::widths`,
  `QueryTab::widths`) and are recomputed when the column set changes. Numbers
  are right-aligned using the declared type.
- **Type-aware editors.** `db::FieldEditor::from_decl` maps declared types to
  editors: booleans (`bool`, `boolean`, `tinyint(1)`) toggle with a checkbox
  (double-clicking the cell flips the value), `date`/`timestamp`/`datetime`
  fields get a month calendar popup, `json`/`jsonb` open the full value window
  with a Format button, and blob types (`blob`, `bytea`, `binary`) open a
  read-only hex dump with a text preview. Closed value sets become dropdowns:
  MySQL `enum(...)` column types, PostgreSQL enum labels read from `pg_enum`,
  and SQLite `CHECK (column IN (...))` constraints. The row detail panel uses
  the same editors, with a Pretty/Raw toggle for JSON and Copy hex/Copy text
  for blobs. Binary editing is not supported because the row pipeline is
  string-based; blobs are shown as `0x…` hex capped at 2 KB.
- **SQL editor intelligence.** While the editor has focus, typing at least two
  characters opens a completion popup anchored at the caret: table names, view
  names, cached column names and SQL keywords, ranked columns/tables first.
  Up/Down move, Tab accepts, and column names are cached per connection.
  `⇧⌘F` (or Format in the header) reflows the statement — clauses on their own
  line, keywords uppercased, literals untouched — through `sqlformat`-style
  token rules in `src/sqlfmt.rs`. The Templates menu inserts SELECT/JOIN/
  INSERT/UPDATE/DELETE/CREATE TABLE snippets at the caret. `:name` parameters
  open a value dialog and are substituted as dialect-appropriate literals
  (numbers, booleans and NULL stay unquoted; text is escaped). Explain runs
  `EXPLAIN QUERY PLAN` on SQLite and `EXPLAIN` elsewhere. Engine errors with a
  line number (PostgreSQL `LINE n:`, MySQL `at line n`) mark that gutter line in
  red and add a Line n chip that jumps the caret there. The divider between the
  editor and the results is draggable; the ratio is stored per tab and seeded
  from Settings.
- **Native menu and shortcuts.** On macOS, `src/menu.rs` installs an AppKit
  menu bar through `muda` (Dataloom, File, Edit, Query, View, Help) with
  accelerators for New Connection (⌘N/⌘K), New Query (⌘T), Close Tab (⌘W),
  Duplicate (⇧⌘T), Save (⌘S), Export/Import SQL (⇧⌘E / ⇧⌘I), Settings (⌘,),
  Undo/Redo (⌘Z / ⇧⌘Z), Find (⌘F), Command Palette (⇧⌘P), Run (⌘Enter), Explain
  (⌘E), Format (⇧⌘F), theme (⇧⌘D), zoom (⌘=/⌘-/⌘0) and About. The same commands
  are reachable from the ⇧⌘P palette, which fuzzy-filters by label or shortcut.
  The egui key handler and the menu both dispatch through
  `dispatch_command`; a 200 ms dedupe guard prevents a double action when both
  fire. ⌘F focuses the table's Quick Filter or the query's result search; ⌘Z
  undoes the last cell/row-detail edit (a journal of table + key + column, so
  it survives tab switches), while text fields keep egui's own undo.
- **Persistent settings.** `Settings` (in `src/settings.rs`) stores theme, data
  and SQL font sizes, row height, alternating rows, row numbers, tab width, the
  default editor split, session restore and close confirmation in
  `settings.json`. Settings changed in the Settings window apply immediately
  and are written on every change; a hand-edited file is clamped on load. The
  grid reads row height, font size, alternating rows and row numbers from these
  values every frame.
- **Session restore and unsaved changes.** Every two seconds (and on Cmd+S,
  disconnect and close) the app writes `session.json`: the connection id and
  name, the active tab and each tab's type. Query tabs keep their title, SQL and
  row limit; table tabs keep schema/name, page offset/limit, applied filter,
  sort terms, row-detail state and the grid's vertical scroll offset, which is
  reapplied on the first frame. On launch, `restore_session` reopens the saved
  connection and tabs when the setting is on, skipping deleted connections or
  tables. Closing the window with unrun editor changes and the confirmation
  setting on intercepts the close (`CancelClose`) and offers Save & quit,
  Discard (clears the session file) or Cancel. An explicit Disconnect clears
  the session so a disconnected workspace is not restored.
- **Read-only connections.** Each saved connection has a `read_only` flag,
  editable in the connection form and toggled from the toolbar chip, the View
  menu or `⇧⌘L`. The flag is stored on the connection and applied to both the
  UI connection and every worker request, so it takes effect without
  reconnecting. `db::run_sql` refuses any statement classified as a write
  (`db::statement_writes` scans keywords outside strings/comments, ignores
  `FOR UPDATE` and plain `EXPLAIN`, and checks every statement in a script),
  and the public write methods (cell/row updates, inserts, deletes, DDL,
  truncate, drop, clone, SQL import) return a read-only error as defense in
  depth. The UI disables cell editing, Add/Delete record, structure edit/delete,
  destructive table-menu items, data import and backup restore, and shows a
  `read-only` chip in the toolbar and status bar.
- **Safe mode.** A global setting that intercepts write statements from the SQL
  editor and shows the statement before running it (Safe mode window with Run /
  Cancel). Reads, `EXPLAIN` and statements inside an already-open transaction
  run without the prompt. It is independent from read-only mode: safe mode
  confirms, read-only refuses.
- **Editor transactions.** Begin/Commit/Rollback run on the query worker's
  connection, which is where the SQL editor executes. While a transaction is
  open, every statement run from the editor joins it; the query header shows
  the elapsed time and statement count, and Commit/Rollback are also in the
  Query menu and the command palette. A failed COMMIT or ROLLBACK clears the
  indicator; cancelling a transaction command is reported. Disconnecting,
  reconnecting or closing the window drops the worker and therefore rolls the
  transaction back; closing with an open transaction warns first. Cell edits,
  row inserts/deletes and DDL still use the UI connection and auto-commit, so
  they are deliberately outside the editor transaction—the header labels and
  tooltips say "editor transaction" to keep that boundary visible.
- **Data import (CSV / Excel / JSON).** File → *Import data…* (or dropping a
  file on the window) opens a dialog that parses the file, maps its columns and
  previews the first rows. CSV/TSV is read by a hand-written RFC 4180 parser
  with delimiter sniffing (comma, tab, semicolon, pipe), quoted fields, doubled
  quotes and newlines inside quotes; the header row is detected (no duplicate
  names, no numeric first cell) and can be forced on or off. JSON accepts an
  array of objects (union of keys, nested values serialized compactly), an
  array of arrays, a single object, or a one-array wrapper such as
  `{"data":[...]}`. Excel (`.xlsx`/`.xls`/`.ods`) is read with `calamine`: first
  sheet by default, sheet picker, numbers/bools/dates converted to text, error
  cells reported as warnings. The dialog can create a new table (column names
  sanitized, types inferred as INTEGER/REAL/TEXT/BOOLEAN per dialect) or append
  to an existing table (mapping by name, then by position; any source column can
  be skipped or pointed at another target). Options cover NULL tokens
  (case-insensitive, default `NULL` and `\N`), empty-string-as-NULL, trimming,
  batch size and deleting existing rows first. Imports run on a background
  thread with its own connection, inside `BEGIN`/`COMMIT`: batches are inserted
  as multi-row statements with literals quoted by target type, progress is shown
  with a cancel button, and any failure rolls the whole import back (including
  the delete). A failed import reopens the dialog with the mapping intact so it
  can be retried. A successful import refreshes the schema and opens the target
  table. Large files are held in memory; binary blobs, formulas with cached
  values only, and per-cell type overrides are not supported.
- **TLS for network drivers.** MySQL and PostgreSQL connections can use
  `Disable`, `Require`, `Verify CA` or `Verify full` SSL modes. `Require`
  encrypts without checking the certificate; the verify modes validate the
  chain, and `Verify full` also checks the host name. An optional PEM/DER root
  certificate can be supplied, otherwise the system trust store is used. MySQL
  uses `mysql::SslOpts` over native-tls; PostgreSQL uses `postgres-native-tls`.
  `Verify full` is rejected when an SSH tunnel is enabled because the tunnel
  connects to `127.0.0.1`; use `Verify CA` in that case. Client certificates are
  not exposed yet.
- **SSH tunnels.** The connection form can route MySQL/PostgreSQL through the
  system `/usr/bin/ssh`: Dataloom reserves a local port, runs
  `ssh -N -T -L 127.0.0.1:<local>:<db-host>:<db-port> … user@bastion` with
  `BatchMode=yes`, `ExitOnForwardFailure=yes` and
  `StrictHostKeyChecking=accept-new`, waits for the forwarded port, and keeps
  the child process in `Live` until disconnect. It reads `~/.ssh/config` and the
  agent, and accepts an optional private-key path. Password-only SSH is not
  supported because BatchMode never prompts; tunnel failures include the tail of
  the ssh stderr log. The first connection accepts an unknown host key. The
  test suite covers the ssh argument construction and TLS mode mapping; it does
  not open a live TLS or SSH endpoint here.
- **Keychain-backed passwords.** Network passwords live in the macOS Keychain
  through Security.framework (service `com.dataloom.app`, account
  `connection-<id>`). `Store::save` moves any plaintext password into the
  Keychain and clears it from `connections.json`; `Store::load` migrates legacy
  files, `connect_saved` loads the secret, `duplicate_connection` copies it to
  the new id, deleting a connection removes it, and exports never include it.
  Leaving the password blank while editing keeps the existing Keychain item. If
  the Keychain is unavailable (locked, denied by a sandbox, or non-macOS), the
  password stays in the local JSON so the connection still works; that fallback
  is the only case where a password is written to disk.
- **Read/write edits.** Double-clicking a cell in a table tab opens an editor.
  The update targets `rowid`, so views and `WITHOUT ROWID` tables cannot be
  edited in place.
- **Delete confirmation.** Dataloom asks before removing a record. The original
  `TableContentTable deleteRow:` path (`0x1005c902c` → `sub_1005c6fc8`) deletes
  the selected rows immediately and relies on `NSUndoManager` for recovery; this
  build keeps a confirmation step and has no undo history yet.
- **Errors are honest.** A failed statement shows the SQLite message in the
  result pane and in a toast; it never pretends to have succeeded.
- **Current-query scope.** Dataloom runs every statement in the editor for both
  Cmd+Enter and Cmd+Shift+Enter. The original runs the statement under the cursor
  for Cmd+Enter; that cursor-scoped distinction is not reproduced.
- **In-cell editing.** Double-clicking a cell swaps the painted text for a
  `TextEdit` in the same grid cell. Enter (or clicking away) writes the value
  through `update_cell` and refreshes the page; Escape discards the edit. An
  untouched NULL cell stays NULL (it is not silently converted to an empty
  string), while clearing a non-NULL value writes `''`. The cell's right-click
  menu offers Edit cell, Set NULL, Set empty and Copy value, mirroring the row
  detail menu, plus Copy selection for a multi-cell range; Set NULL is
  disabled on NOT NULL and primary-key columns. Views
  and tables without a key column stay read-only. The original's cell editor is
  native; this one is an egui widget placed by the custom grid.
- **Grid sorting, selection and copy.** Clicking a column header sorts it
  ascending, then descending, then clears the sort; Shift-click adds or toggles
  extra columns for a multi-column sort. The arrows show the direction and, for
  multi-column sorts, the priority. Table tabs turn the sort into a server-side
  `ORDER BY` (with the row key as a stable tiebreaker); query-result tabs sort
  the loaded rows in memory, numbers as numbers and NULLs last. Clicking a cell
  selects it, Shift-click or Shift+arrows extends a rectangular range, clicking
  a row number selects the whole row and Shift-click on the gutter extends a row
  range. Arrow keys and Tab move the cursor, Cmd+A selects everything, Cmd+C
  copies the selection as TSV, and Enter or F2 opens the cell editor. The
  table footer shows `rows×columns selected` for multi-cell selections. The
  row-detail panel follows the cursor row.
- **Virtualized rows.** The data grid reserves the full content height and
  width (so the scrollbars are correct) but paints and hit-tests only the rows
  intersecting the scroll viewport, plus one row of overscan on each side. A
  100,000-row result therefore does not create 100,000 widgets or text runs.
  The row pitch is exactly 22 pt: `NSRowHeight` 22 with the nib's zero
  intercell spacing. Keyboard moves that leave the viewport scroll the cursor
  cell into view. The header scrolls vertically with the content; a sticky
  header is not implemented.
- **Row detail editor.** The right sidebar shows the selected record as a
  vertical, searchable field form, matching `RightSidebarController` /
  `RowDetailTable` (classes, frames and strings decoded in
  `ROW_DETAIL_EVIDENCE.md`). It is available on table Data tabs, starts open,
  and toggles from the main toolbar's split-pane button — shown as a labelled
  `Row detail` control while the panel is closed, like the original's Panels
  control — or from the space bar. Like TablePlus's toolbar toggle it stays
  visible so the panel can always be brought back. The panel shows the nib's
  `No row selected` empty state.
  Typing saves a field when it loses focus or on Enter; TablePlus instead stages
  edits behind a toolbar Commit control (Cmd+S) with discard and SQL-preview
  segments. Dataloom does not reproduce that staging, Undo, the Assistant tab
  or `DEFAULT` conversion. Query-result grids do not open the panel yet.
- **SQL tools instead of wizards.** The `SQL tools` window exports a table or the
  whole database to a `.sql` file (`CREATE` plus one `INSERT` per row, values
  quoted by the database itself), imports a `.sql` file statement by statement,
  and keeps timestamped backups. The export and import paths can be typed, or
  picked with the native macOS open/save panels provided by the `rfd` crate
  (`Browse…` / `Save as…`); the panels are modal, so the app pauses while one is
  open. The table header's `Export` menu exports the current table view as CSV,
  Excel or SQL with every row that matches the applied filter (not just the
  visible page); the SQL tools window offers the same filter as a checkbox.
  Unlike the original's export, Dataloom writes no `DROP TABLE IF EXISTS` and no
  gzip; imports run without a wrapping transaction.
- **Backups.** Each connection gets a folder under the app data directory.
  SQLite is backed up as a byte-for-byte file copy and can be restored in place
  (the connection is closed, the file replaced and reopened); MySQL/PostgreSQL
  are backed up as SQL dumps, which are restored through Import. The original
  schedules backups through `BackupWindow`/`BackupRunner`; Dataloom only makes
  them on demand.
- **PostgreSQL schemas.** Tables and views in the connection's current schema
  keep their bare names; relations in other schemas are shown as
  `schema.table`. The internal `TableId` keeps schema and name separate, so a
  table whose own name contains a dot is not mistaken for a qualified one. Data,
  structure, indexes, DDL, edits, exports and clones all qualify the relation.
- **Not carried over.** Backup schedules, AI chat, metrics boards and the MCP
  server are visible in the original but are outside this workspace.
  Query history is
  written to `query_history.json` in the app data folder; the original keeps it
  per connection, while Dataloom stores one global file.

## Code map

| Concern | Location |
| --- | --- |
| App shell, chrome, tabs, sidebar, screenshot mode | `src/app.rs` |
| Query editor, result pane, table browser, dialogs | `src/views.rs` |
| Data grid, SQL highlighting, small painted controls | `src/widgets.rs` |
| SQLite access, paging, filtering, structure, sample data, table ops | `src/db.rs` |
| SSH local port forwarding | `src/tunnel.rs` |
| Preferences and last session | `src/settings.rs` |
| Commands, macOS menu bar, shortcut labels | `src/menu.rs` |
| SQL formatting, parameters, error lines, completion | `src/sqlfmt.rs` |
| CSV/JSON/Excel parsing and background import | `src/import.rs` |
| Dependency-free XLSX workbook writer | `src/xlsx.rs` |
| Dark/light palette and typography | `src/theme.rs` |
| Machine-readable values | `mechanics.json` |
| Static evidence for the row detail sidebar | `ROW_DETAIL_EVIDENCE.md` |

## Constants that must stay aligned with `mechanics.json`

- `db::DEFAULT_ROW_LIMIT` = 0, "No limit" (`QueryController.nib` item 367; no stored key)
- `db::MAX_ROW_LIMIT` = 10,000,000 (`db::validate_paging`, `sub_10071b5e4`)
- `db::validate_paging` mirrors the four messages and the exact `offset != 0 && offset >= total` rule from `sub_10071b5e4`
- `db::parse_paging_text` accepts an optional sign and decimal digits only; `views::paging_window` uses it, matching `sub_100532a80` (invalid text, including whitespace, becomes 0)
- `views::paging_window` shows `db::page_step(tt.limit)` = `max(limit, 300)` in the Limit field, matching `sub_10081f074`
- the table footer and status bar use `Rows (loaded of total loaded)` when the total is known and `Rows (N so far)` when it is not, from `sub_10000e300`
- the table footer uses `Only N rows loaded` from `sub_100018d60` when the page is capped
- the table grid shows `No matching columns or rows` from `sub_10000e300` while a filter is active
- Quick Filter lowercases both the column and the pattern (`sub_1006f95a0`), including the SQLite/MySQL/PostgreSQL SQL paths
- `db::TableFilter` merges the Quick Filter with the column filters; `db::FilterOp` covers contains, does not contain, starts with, ends with, equals, not equals, is empty and is not empty. `build_where` emits bound `?` parameters for SQLite/MySQL and escaped literals for PostgreSQL (its helper only runs literal SQL); `any` switches AND to OR
- `app::ROW_LIMIT_OPTIONS` mirrors the nine original menu titles (`QueryController.nib` items 367–383; menu tags are 1–8, not the values)
- `db::DEFAULT_TABLE_PAGE_LIMIT` = 300, offset = 0 (`open_table_tab`; `sub_10071b144` / `sub_10071b370` / `sub_10081f074` fall back to `0x12c`)
- `db::MIN_PAGE_STEP` = 300 and `db::page_step(limit) = max(limit, 300)`; next/previous buttons, Cmd+Left/Right and the status bar all use it
- window size = 1400 × 880, minimum 980 × 620 (`src/main.rs`)
- sidebar width = 297 (`app::SIDEBAR_W`; saved `WorkspaceSplitView` frame is 296.5)
- `theme::DATA_FONT_SIZE` = 11, `theme::GRID_ROW_HEIGHT` = 22 (`TableDataController.nib` object 106, `NSRowHeight`; used by `widgets::data_grid`); `DataRowPadding` 12 is recorded in `mechanics.json` but is not the row height
- `theme::SIDEBAR_FONT_SIZE` = 13, `theme::SQL_FONT_SIZE` = 11, `theme::LINE_HEIGHT` = 1.2
- `theme::EDITOR_LINE_HEIGHT` = 13.2 = 11 × 1.2 (`widgets::highlight_sql`)
- `theme::TAB_WIDTH` = 4, `theme::TAB_WIDTH_OPTIONS` = [2, 4, 6, 8] (`sub_1008168c0`)
- the status bar shows `theme::TAB_WIDTH`; the editor keeps literal tabs
- `theme::DEFAULT_DARK` = false: every launch opens the light palette whatever the system appearance is; the toolbar toggle switches to dark (`cc.egui_ctx.set_theme(ThemePreference::Light)`). The original follows the system (`ViewSetting` → `ThemeID` = `builtin:auto`)
- accent = `#4F8CFF` (`theme::Palette::dark`; recreation palette, not sampled)
- `app::ROW_DETAIL_W` = 300 (`RightSidebarController.nib`: 300 pt detail cells) and `app::ROW_DETAIL_MIN_W` = 240 (nib width constraint 240 at priority 750); `views::row_detail_panel` defaults to `ROW_DETAIL_W + 36`
- row detail UI strings are the original's: `Search for field`, `No row selected`, `Fuzzy - default`, `Contains`, `NULL`; field type labels are `long` / `var_string` / `blob` (MySQL native type names)
- `widgets::data_grid` selection drives `views::sync_row_detail`; edits go through `db::Db::update_field`, which writes SQL NULL for `None` (row detail chevron) and an empty string for `Some("")`
- the row detail toggle is `app::DataloomApp::toggle_row_detail` (toolbar split-pane button and Space, matching the nib tooltip); it is only active on table Data tabs
