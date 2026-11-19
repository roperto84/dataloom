# TablePlus Row Detail (right sidebar) — static evidence sheet

Scope: **static inspection only** of the installed original app; the app was never launched.
Installed build: `/Applications/TablePlus.app` — `CFBundleShortVersionString = 27.3.2`,
`CFBundleVersion = 850`, bundle id `com.tinyapp.TablePlus` (from `Contents/Info.plist`).

Primary sources and how they were read:

| Source | Format / method |
|---|---|
| `Contents/Resources/RightSidebarController.nib` | compiled `NIBArchive` (magic `NIBArchive`, header: 691 objects / 49 class names / 237 keys / 3476 values). Parsed with the `nibarchive` 1.1.0 Python parser (format doc: <https://github.com/matsmattsson/nibsqueeze/blob/master/NibArchive.md>); object indexes below are indexes in that archive. Cross-checked with `strings -a`. |
| `Contents/Resources/WorkspaceWindow.nib` | same `NIBArchive` parser + `strings -a`. |
| `Contents/Resources/en.lproj/Localizable.strings` | UTF-16LE key/value table (1438 pairs), parsed with Python. |
| `Contents/MacOS/TablePlus` | universal binary; arm64 slice extracted with `lipo -thin arm64`; `strings -a`, `otool -v -s __TEXT __objc_methname` (offsets below are arm64 virtual addresses). |
| `Contents/Frameworks/MySQL97.framework`, `SQLite.framework` | `strings -a`, for native type-name evidence. |

Object references are written as `#<index>` (nib object index) and `selector @0x…` (method-name offset).

---

## 1. UI classes, outlets and actions

### 1.1 Classes that make up the row detail panel

| Finding | Evidence |
|---|---|
| Sidebar controller is a Swift class `_TtC9TablePlus22RightSidebarController`; the nib file-owner custom object `#2` has `NSClassName = _TtC9TablePlus22RightSidebarController` | `RightSidebarController.nib` obj #2–#7 (`NSClassName` NSString `_TtC9TablePlus22RightSidebarController`) |
| Row list is a custom `NSTableView` subclass **`RowDetailTable`** (`_TtC9TablePlus14RowDetailTable`) | obj #92: `NSClassSwapper`, `NSClassName = _TtC9TablePlus14RowDetailTable`, `NSOriginalClassName = NSTableView`; Swift metadata string `RowDetailTable` @ main binary |
| Field cells are **three** custom `NSTableCellView` subclasses: `RowDetailTextCell`, `RowDetailByteaCell`, `RowDetailDropdownCell` | objs #448/#410/#290; reuse ids `RowDetailTextCell` / `RowDetailDropdownCell` / `RowDetailByteaCell`; registered in table's `NSTableViewArchivedReusableViewsKey` dictionary obj #102 (key→`NSNib`→nested `NIBArchive` values) |
| Value editor text view is a custom `NSTextView` subclass **`RowDetailTextView`** | obj #460 (+ obj #31 in the nested TextCell prototype): `NSClassSwapper`, `NSClassName = _TtC9TablePlus17RowDetailTextView` |
| Dropdown editor is a custom `NSPopUpButton` subclass **`RowDetailPopupButton`** | obj #423: `NSClassName = _TtC9TablePlus20RowDetailPopupButton` |
| Search field cell is a custom `NSSearchFieldCell` subclass **`MenuSearchFieldCell`** | obj #129: `NSClassName = _TtC9TablePlus19MenuSearchFieldCell`, `NSOriginalClassName = NSSearchFieldCell` |
| Tab-header buttons use `AutoSizeSidebarButton`; the small in-cell menu chevron uses `AutoSizeButton` | objs #20/#34 (`_TtC9TablePlus21AutoSizeSidebarButton`), obj #471 (`_TtC9TablePlus14AutoSizeButton`) |
| Assistant tab uses `RowChatSystemCell`, `RowChatUserCell`, `AIMessageSystemTextView`, `AIMessageUserTextView` | objs #213/#370/#246/#387 (out of scope for row detail, kept for completeness) |
| Binary class/protocol Swift type names (main binary `strings`) | `RowDetailTextView`, `RowDetailTable`, `RowDetailTableDelgate` (sic), `RowDetailTextCell`, `RowDetailByteaCell`, `RowDetailByteaCellDelegate`, `RowDetailDropdownCell`, `RowDetailDropdownCellDelegate`, `RowDetailCellDelegate`, `RowDetailPopupButton`, `RowDetailValueSearch`, `MenuSearchFieldCell`, `AutoSizeButton`, `AutoSizeSidebarButton` |

### 1.2 Outlets (all `NSNibOutletConnector`s; source `#2` unless noted)

| Outlet | Target (class #obj) | Evidence |
|---|---|---|
| `view` | `NSView` #208 | connection label `view`; obj #208 `NSFrameSize = {300, 600}` |
| `buttonSegment` | `NSView` #32 | connection label `buttonSegment`; #32 frame `{{0, 563}, {300, 37}}` |
| `detailsButton` | `AutoSizeSidebarButton` #34 | label `detailsButton`; button cell contents `'Details'` |
| `assistantButton` | `AutoSizeSidebarButton` #20 | label `assistantButton`; button cell contents `'Assistant'`; `NSTag = 1` |
| `tabView` | `NSTabView` #149 | label `tabView`; frame size `{300, 564}` |
| `rowSearchField` | `NSSearchField` #127 | label `rowSearchField`; frame `{{10, 552}, {254, 24}}` |
| `rowDetailTable` | `RowDetailTable` #92 | label `rowDetailTable`; frame size `{300, 542}` |
| `rowLabel` | `NSTextField` #114 | label `rowLabel`; frame `{{8, 270}, {284, 73}}` |
| `aiChatView` | `NSView` #11 | label `aiChatView`; frame `{{8, 8}, {284, 90}}` |
| `aiChatViewHeightConstraint` | `NSLayoutConstraint` #15 | label `aiChatViewHeightConstraint`; firstItem #11, attribute 8 (height), `NSConstant = 90.0` |
| `buttonSegment` children | two `AutoSizeSidebarButton`s (#34 details, #20 assistant) | outlet labels above + subview arrays #33/#23 |
| `column` data cell | `NSTextFieldCell` `'Text Cell'` (#101) | obj #99 `NSDataCell`; contents string `'Text Cell'` |

Cell-level outlets (unarchived from the reusable-view prototypes):

| Cell | Outlets | Evidence |
|---|---|---|
| `RowDetailTextCell` (#448) | `fieldName` → `NSTextField` #451; `fieldType` → `NSTextField` #456; `textView` → `RowDetailTextView` #460; `setConstantButton` → `AutoSizeButton` #471 | outlet connections; same set repeated in nested TextCell archive |
| `RowDetailByteaCell` (#290) | `fieldName` → #293; `fieldType` → #298; `imageView` → `NSImageView` #303; `textField` → `NSTextField` #319; `heightConstraint` → `NSLayoutConstraint` #316 (firstItem #303, attribute 8, `NSConstant = 184.0`) | outlet connections |
| `RowDetailDropdownCell` (#410) | `fieldName` → #413; `fieldType` → #418; `popupButton` → `RowDetailPopupButton` #423 | outlet connections |
| `RightSidebarController` | `menu` outlet from plain `NSButton` #153 → `NSMenu` #344 | outlet label `menu`; #153 is the search-options button (image `preferencesTemplate`, 17×14) |

### 1.3 Actions (all `NSNibControlConnector`s)

| Action selector | Sender → target | Evidence |
|---|---|---|
| `changeTab:` | `detailsButton` #34 → RightSidebarController #2; `assistantButton` #20 → #2 | two control-connector records; details has tag 0, assistant `NSTag = 1` |
| `filterRows:` | `rowSearchField` #127 → #2 | record label `filterRows:` (selector `filterRows:` @0x10115b810) |
| `searchPreferences:` | `NSButton` #153 → #2 | record label `searchPreferences:` @0x10116fe78 |
| `updateSearchStrategy:` | menu items #359 (`Fuzzy - default`) and #361 (`Contains`) → #2 | two records; @0x10118344b |
| `enableLLMChat:` | menu item #363 (`Enable LLM Chat`) → #2 | @0x1011597b7 |
| `enablePrettyJSON:` | menu item #366 (`Auto Pretty JSON`) → #2 | @0x1011598c6 |
| `showMenu:` | `AutoSizeButton` #471 → `RowDetailTextCell` #448; `NSButton` #324 → `RowDetailByteaCell` #290 | two records; @0x10117c932 |
| `popupButtonDidSelect:` | `RowDetailPopupButton` #423 → `RowDetailDropdownCell` #410 | @0x10116b5dc |
| `copyAction:`, `deleteAction:`, `sendAction:` | report-chat cells only (`RowChatSystemCell` #213 / `RowChatUserCell` #370) — **not** row-detail cells | connectors; selectors @0x101154a7a, @0x101156b10, @0x101170a16 |

Controller-side properties/selectors found in the binary (objc method/property names):

`rowDetail`, `rowDetailTable`, `rowDetailTableDelegate`, `rightSidebarController`, `rowLabel`, `rowSearchField`,
`rowTargets`, `rowSearchMatchCase`, `rowSearchMatchWholeWord`, `rowSearchUseRegularExpression`,
`setRowDetail:`, `setRowDetailTable:`, `rightSidebarController`, `isEnableAutoPrettyJSON`,
`toggleRightSidebar:`, `toggleSplitView:`, `toggleBottomSidebar:`, `toggleAssistant:`, `toggleEnableAssitantTab:`, `changeMatchingStrategy:`.

Also present: type `RowDetailValueSearch` (Swift metadata string) and `SQLColumnScore`/`SQLRowScore` — exact role not recoverable statically.

---

## 2. Visible strings / labels

| Finding | Evidence (exact string + source) |
|---|---|
| Search placeholder: **`Search for field...`** | obj #133 NSString `NS.bytes = b'Search for field...'`, set as `NSPlaceholderString` on `MenuSearchFieldCell` #129. `Localizable.strings` also has key `"Search for field"` (lookup key without dots). |
| Search options menu attached to the search row (`preferencesTemplate` sliders button) | `NSMenu` #344, connected from button #153 via `menu` outlet |
| Empty state: **`No row selected`** | `NSTextFieldCell` #117 `NSContents = 'No row selected'` (label `rowLabel` #114); 15 pt `.AppleSystemUIFont`, text color `secondaryLabelColor`, frame `{{8, 270}, {284, 73}}`. The literal exists **only in the nib** (0 occurrences in the main binary). |
| Tab item labels: **`Row`**, **`Assistant`** | `NSTabViewItem` #184 `NSLabel = 'Row'` (selected item, view #151); `NSTabViewItem` #186 `NSLabel = 'Assistant'` (view #187); tab-item array #183 = `[184, 186]`; `NSTabView` #149 `NSSelectedTabViewItem = #184` |
| Header button titles: **`Details`**, **`Assistant`** | `NSButtonCell` #37 `NSContents = 'Details'`; `NSButtonCell` #25 `NSContents = 'Assistant'` |
| Matching-strategy menu: parent item **`Matching strategy`**, submenu items **`Fuzzy - default`** (checked) and **`Contains`** | `NSMenu` #344 items `[346, 363, 365, 366]`; #346 title `'Matching strategy'`, action `submenuAction:`, submenu `NSMenu` #357; #359 title `'Fuzzy - default'`, `NSState = 1`; #361 title `'Contains'`, `NSTag = 1` |
| Other entries of the same options menu: **`Enable LLM Chat`**, **`Auto Pretty JSON`**, one blank item | #363 `'Enable LLM Chat'`; #366 `'Auto Pretty JSON'`; #365 title `''`, `NSIsSeparator = False` (blank placeholder item) |
| Dropdown cell popup options: **`EMPTY`**, **`NULL`**, **`DEFAULT`** | `NSMenu` #433 items `[431, 435, 436]` = `'EMPTY'`, `'NULL'`, `'DEFAULT'`; `NSPopUpButtonCell` #428 `NSContents = 'EMPTY'`; all items action `_popUpItemAction:` |
| IB placeholder text inside cell prototypes (design-time samples, replaced at runtime) | Bytea: fieldName `'bytes'` (#296), fieldType `'bytea'` (#301), value textField contents `'PNG'` (#322) with placeholder **`'NULL'`** (#323). Dropdown: `'dropdown'`/`'boolean'` (#416/#421). Text: `'text'`/`'text'` (#454). Table data cell default `'Text Cell'` (#73). |
| Menu-chevron image used by both in-cell menu buttons | `DownArrowTemplate` NSImage, 14×14 (`NSCustomResource` #329 → `'downArrowTemplate'`, `IBDesignSize = {14, 14}`) inside 20×18 buttons |
| Theme/localizable hooks around row detail | `Localizable.strings`: `"Row detail text"`, `"Details"`, `"Row"`, `"Search for field"`, `"Search Options"`, `"Enable Assistant Tab on the Right Sidebar"`, `"When you enable the Assistant Tab, you will be able to ask the AI to write queries for you."`, `"Read Only"`, `"Read-only"`, `"“%@” is read-only."`; binary theme color names `gridRowDetailText` / `GridRowDetailText` (row-detail text color) |
| Strings *not* attributable to row detail (checked to avoid mis-citation) | `"Convert NULL to EMPTY"` is an **export** option (present in `Export*Window.nib`, absent from main binary strings); `"Use \">\" to search in the column content"` and `"No matching columns or rows"` sit in the main-grid search/replace string block, not proven to be row detail. |

---

## 3. Geometry recorded in the nibs (all values are design-time; class `NSFrame`/`NSFrameSize` strings)

| Element | Value | Evidence |
|---|---|---|
| Row detail pane root view | `{300, 600}` (width × height), origin implied `{0,0}` | `RightSidebarController.nib` obj #208 `NSFrameSize = '{300, 600}'` |
| Header strip | `{{0, 563}, {300, 37}}` — full width, 37 pt tall | obj #32 `buttonSegment` `NSFrame` |
| `Details` header button | `{{79, 9}, {65, 19}}` | obj #34 |
| `Assistant` header button | `{{154, 9}, {65, 19}}` | obj #20 |
| Header buttons centering | pair spans x 79…219 in a 300-wide strip → 10 pt gap, horizontally centered | derived from the two frames above |
| Tab view | size `{300, 564}` | obj #149 `NSFrameSize` |
| Row tab content view | size `{300, 564}` | obj #151 `NSFrameSize` |
| Row search field | `{{10, 552}, {254, 24}}` | obj #127 `NSFrame` |
| Search-options (sliders) button | `{{263, 542}, {35, 22}}`, image `preferencesTemplate` 17×14 | obj #153 `NSFrame`; image resource #157/#159 |
| Field list scroll view | size `{300, 548}`; clip view bottom content inset `'{0, 0, 6, 0}'` | objs #162 `NSFrameSize`, #167 `NSContentInsetsName` |
| Row list table | size `{300, 542}`; row height 230 (design default); intercell spacing 0×0 | obj #92: `NSFrameSize`, `NSRowHeight = 230.0`, `NSIntercellSpacingWidth/Height = 0.0` |
| Table column | width 288, min 40, max 10000, not resizable, not editable; header cell empty | obj #99 `NSTableColumn` (`NSWidth = 288.0`, `NSMinWidth = 40.0`, `NSMaxWidth = 10000.0`, `NSIsResizeable = False`, `NSIsEditable = False`); header cell #100 `NSContents = ''` |
| Table selection style | no selection highlight; allows type-select | obj #92 `NSTableViewSelectionHighlightStyle = -1`, `NSAllowsTypeSelect = True` |
| Empty-state label | `{{8, 270}, {284, 73}}` | obj #114 |
| **RowDetailTextCell** (prototype) | `{300, 37}` | obj #448 `NSFrameSize`; nested archive reuse id `RowDetailTextCell`, framesize `{300, 37}` |
| — fieldName / fieldType inside text cell | x 10/w 175 and x 180/w 110, y 16, h 17 | objs #451, #456 |
| — value text view inside text cell | `{{10, -7}, {280, 14}}` | obj #460 |
| — menu chevron button inside text cell | `{{268, -10}, {20, 18}}` | obj #471 |
| **RowDetailDropdownCell** (prototype) | `{{0, 37}, {300, 44}}` | obj #410; nested archive same |
| — fieldName / fieldType | y 23, h 17 (x 10/175 and 180/110) | objs #413, #418 |
| — popup button | `{{10, 4}, {280, 24}}` | obj #423 |
| **RowDetailByteaCell** (prototype) | `{{0, 81}, {300, 230}}` | obj #290; nested archive same |
| — fieldName / fieldType | y 209, h 17 | objs #293, #298 |
| — image preview | `{{12, 25}, {276, 184}}`, height constraint `184.0` | obj #303; obj #316 |
| — format/value text field | `{{10, 4}, {242, 19}}` | obj #319 |
| — menu chevron button | `{{268, 2}, {20, 18}}` | obj #324 |
| Cell style | labels/type: 11 pt system font, `secondaryLabelColor`, background `controlColor`; rowLabel 15 pt; popup 13 pt; search 12 pt | cell/font objs #295/#300/#415/#420/#453/#458, #119, #132, #430 |
| Layout constraints (row tab) | search leading = +10, search trailing = −36, search top = content top; scroll top = search bottom + 4; scroll leading/trailing/bottom = content edges | constraint array #175, items #176–#182 (`NSConstant` 36/10/4, attributes 5/6/3/4) |
| **Frame vs constraint inconsistency (flag)** | search frame `{{10,552},{254,24}}` has its top at 576 pt but the tab content frame height is 564 pt → the IB frame was laid out taller than the recorded content size; at runtime the autolayout constraint “search top = content top” governs. Treat the y=552 number as design residue, not runtime truth. | objs #127, #151 (`NSFrameSize = {300, 564}`), constraint #178 |
| Layout constraints (dropdown cell) | popup leading 10, trailing 10, top 20, bottom 4 (constant/frame mismatch vs 44 pt frame → treat as estimate) | #439 items #440–#443 |
| Layout constraints (text cell) | textView leading 10, trailing 10, bottom 3 | #478 items #479–#481 |
| Workspace window | window rect `{{780, 405}, {1000, 600}}`; content root 1000×600 | `WorkspaceWindow.nib` obj #9 `NSWindowRect`; obj #266 `NSFrameSize` |
| Workspace split view | `ContainerSplitView` size `{1000,600}`, arranged `[tablesListing #303, contentSplitView #307 {{251,0},{498,600}}, rowDetail #398 {{750,0},{250,600}}]` | objs #299, #303, #307, #398, arrays #302/#405 |
| Row-detail container width constraint | width constant **240**, priority **750** (both side panes: tables listing and row detail) | `NSLayoutConstraint` #401 (rowDetail #398) and #306 (tables listing #303), `NSConstant = 240.0`, `NSPriority = 750` |
| Split-view holding priorities | `NSHoldingPriorities` #403 = { arranged-index 0 (tables listing #303) → `489.0`, index 1 (content split #307) → `1`, index 2 (row detail #398) → `489.0` } — the two outer panes resist resizing more than the content pane; arranged subviews #405 = `[303, 307, 398]` | `NSMutableDictionary` #403 raw pairs `[395→396, 351→351, 404→396]` with `NSNumber` #396 `NS.dblval = 489.0`, #351 `intval = 1`; array #405 |

**Width discrepancy (flag):** the right-sidebar content nib is designed **300** wide, the workspace container frame is **250** wide, and the container has a `width ≥/= 240 @750` constraint; the pane is inside a user-resizable `NSSplitView`. Actual runtime width cannot be determined statically (reference screenshot shows a pane wider than 300 pt, i.e. the split view was dragged; estimate 240–320 pt, default ≈300).

---

## 4. Menu item, toolbar control and shortcut that toggle the row detail sidebar

| Finding | Evidence |
|---|---|
| Toggle lives on the workspace toolbar's **`Panels`** segmented control (`NSToolbarItem #164`, `NSToolbarItemLabel = 'Panels'`, view `NSSegmentedControl` #166 frame `{{0, 14}, {65, 24}}`, min/max size `{65,24}`) | `WorkspaceWindow.nib` objs #164, #166 |
| Segment 0 = console/bottom panel: image `bottomSidebarTemplate`, width 30, tooltip **`Toggle console log (⌘⌃C)`** | `NSSegmentItem` #171, image `NSCustomResource` #172→`'bottomSidebarTemplate'` |
| Segment 1 = row detail: image **`rightSidebarTemplate`**, width 34, `NSSegmentItemTag = 1`, tooltip **`Toggle Row Detail (Space)`** | `NSSegmentItem` #177, #178 `NSResourceName = 'rightSidebarTemplate'`; asset present in `Assets.car` (`assetutil --info` → `rightSidebarTemplate.svg`) |
| Control action | `toggleSplitView:` → workspace controller #2 (control-connector record; selector @0x1011822f5) |
| Controller outlet | `toggleSplitViewControl` → `NSSegmentedControl` #166 |
| Row-detail container outlet | `rowDetail` → `NSView` #398 (frame `{{750,0},{250,600}}` inside the container split view) |
| Binary has keyboard/action entry points `toggleRightSidebar:` (@0x10118223e), `toggleSplitView:` (@0x1011822f5), `toggleBottomSidebar:`, `toggleAssistant:`, `toggleEnableAssitantTab:` (sic) | `otool -v -s __TEXT __objc_methname` |
| Space key binding is documented only as the tooltip text `Toggle Row Detail (Space)`; `Localizable.strings` contains the key/value `"Space" => "Space"`; the binary has `AppShortcuts` / `AppShortcutCommand` / `AppShortcut` types. Neither nib contains a menu item that references row detail / `toggleRightSidebar:`; the nibs' menu key equivalents are unrelated and none is Space (checked every `NSMenuItem` in both nibs). | strings/objects above; `WorkspaceWindow.nib` menu items (e.g. `r`, `v`, `i`, `d`, `c`, `k`, `n`, `\r`) — no row-detail item |
| No menu item titled "Toggle Row Detail" exists; the phrase appears **only** as the toolbar-segment tooltip | `WorkspaceWindow.nib` strings: `'Toggle Row Detail (Space)'`; main-binary `strings`: 0 occurrences |
| The workspace window itself is not a repository of row-detail UI: it only has the container view + toolbar segment; `RightSidebarController.nib` is loaded/bound in code (`rightSidebarController` property in binary) | obj #398 has no subviews in `WorkspaceWindow.nib` |

Related toggle/shortcut strings found in `WorkspaceWindow.nib` (for consistency): `'Toggle item list (⌘ 0)'` (help tag #594),
`'Toggle console log (⌘⌃C)'`, `'Refresh workspace (⌘R)'`, `'Open SQL Editor (⌘E)'`, `'Show all workspaces (⌘⇧\\)'`.

---

## 5. How field types are displayed

| Finding | Evidence |
|---|---|
| Every row-detail cell has **two labels**: `fieldName` left, `fieldType` right (175 pt vs 110 pt wide, both 11 pt system, `secondaryLabelColor`). The type label is populated at runtime (design-time strings are IB samples `'bytea'`, `'boolean'`, `'text'`). | cell/obj evidence in §1–2 |
| Nib alignment of the type label is recorded as `NSControlTextAlignment = 2` (center) on #298/#418/#456, while the workspace reference screenshot shows type labels **right-aligned** to the pane edge. Treat nib alignment as a design artifact / runtime override. | objects #298/#418/#456 (`NSControlTextAlignment = 2`); reference screenshot |
| Type labels observed in the reference screenshot: **`long`, `var_string`, `blob`** for MySQL 8 columns (lowercase, underscore). This matches MySQL protocol enum names lowercased (`LONG`, `VAR_STRING`, `BLOB`). | workspace reference `reference_row_editor.webp` (non-static, labeled); `MySQL97.framework` binary contains the enum-name block: `BLOB, VECTOR, BOOLEAN, DATE, DATETIME, NEWDECIMAL, DECIMAL, DOUBLE, ENUM, FLOAT, GEOMETRY, INT24, JSON, LONG, LONGLONG, LONG_BLOB, MEDIUM_BLOB, NEWDATE, NULL, SHORT, STRING, TIME, TIMESTAMP, TINY, TINY_BLOB, VARCHAR, VAR_STRING, YEAR, ?-invalid-?, ?-unknown-?` |
| No row-detail type-label literals exist in the nib, and `var_string`/`VAR_STRING` do not occur in the app binary at all (`grep -aoc` = 0); bare tokens like `long`/`blob` occur only in unrelated SQL/parser data. The label must be computed in code from the driver's column type (lowercased enum name and/or SQL `data_type`/`COLUMN_TYPE`). | main binary string search; nib string listing |
| Driver-side evidence for type names: `SQLite.framework` contains `INTEGER`, `REAL`, `TEXT`, `BLOB` tokens; `MySQL97.framework` contains the enum names above. A Swift type `FormatSQLType` exists in the app binary (metadata string) — likely the formatter, but its enum cases/method names are not recoverable. | framework `strings -a`; main-binary metadata string `FormatSQLType` |
| **Not determined:** exact casing/formatting algorithm (e.g. `var_string` vs `VAR_STRING` vs `varchar(255)`), and how PostgreSQL types are shown. It is *not* the IB sample strings. | — |

---

## 6. Saving / commit model and NULL representation

### 6.1 Commit model (explicit staged commit, not a silent per-field write)

| Finding | Evidence |
|---|---|
| No Apply / Save / Done / OK button exists inside the row-detail nib; the only row-detail buttons are the two header tabs, the search-options button, and per-cell menu chevron buttons | complete outlet/action inventory in §1 (all `NSNibOutletConnector`/`NSNibControlConnector` records) |
| The workspace toolbar has a **`Commit`** item (min/max `{107,24}`) whose view is a 3-segment `NSSegmentedControl` with tooltips: **`Discard all the changes (⌘⇧⌫)`**, **`Preview your changes as SQL Statements (⌘⇧P)`**, **`Commit your changes (⌘S)`** (segment widths 35/35/35; images `discardTemplate`, `previewTemplate`, `commitTemplate`) | `WorkspaceWindow.nib`: `NSToolbarItem` #88 `NSToolbarItemLabel = 'Commit'`, view `NSSegmentedControl` #90 frame `{{0,14},{107,24}}`; segment items #96, #102, #108 |
| That control is wired to the workspace controller as outlet `changeLogControl` with action `changeLogAction:` (nib connection). Naming is legacy/ambiguous, but the tooltips are unambiguous: the UI offers discard / preview-as-SQL / commit actions for pending changes. | connections `changeLogControl` #90; `changeLogAction:` #90→#2; selector @0x101151702 |
| Binary has the corresponding action selectors: `applyChanges:` @0x10114e24a, `commitChanges:` @0x101153420, `discardChange:` @0x101157818, `previewChanges:` @0x10113b3c8; plus editing-end selectors `textDidEndEditing:` @0x101181010 and `controlTextDidEndEditing:` @0x101154710, and standard table delegate selectors `tableView:viewForTableColumn:row:` @0x101180730, `tableView:rowViewForRow:` @0x101180398 | `__objc_methname` |
| Localizable dialog text confirms the explicit commit workflow and `⌘S`: `"Discard all changes?\nTips: You can commit the changes by\n1. Command + S.\n2. Use the top left segment control."`, `"Discard all changes?\nTips: You can save the changes by pressing: Command + S"`, `"Commit"`, `"Discard"` | `en.lproj/Localizable.strings` |
| Pending-change visuals exist as theme colors: `ContentInsertedBackground`, `ContentModifiedBackground`, `ContentDeletedRowBackground`, `GridCellSelectionBackground/Border` | main binary theme-name strings (e.g. metadata/theme block) |
| **Not determined:** whether a cell edit inside the row detail is staged on end-editing and then committed with the global Commit action exactly like a grid edit (the code path is not statically recoverable). The absence of any per-field Apply UI + presence of the global commit workflow is consistent with: *edit is buffered/staged, committed explicitly with ⌘S (or the Commit segment)*; **not** with a per-field auto-write. Treat this as a high-confidence inference. | — |
| Read-only tables: strings exist for `"Read Only"`, `"Read-only"`, `"“%@” is read-only."`, and a no-primary-key warning: `"This change would affect %@ rows, not one: the table has no primary key, and other rows have the same values as the row you changed. Add a primary key, or change these rows with a SQL query."` No read-only UI exists in the row-detail nib → read-only state is code-driven. | `Localizable.strings` |

### 6.2 NULL / EMPTY representation

| Finding | Evidence |
|---|---|
| Dropdown-type cells expose explicit **`EMPTY` / `NULL` / `DEFAULT`** choices and default to `EMPTY` | `NSMenu` #433 items `[431 'EMPTY', 435 'NULL', 436 'DEFAULT']`; `NSPopUpButtonCell` #428 `NSContents = 'EMPTY'`; nested DropdownCell prototype menu `[41 EMPTY, 54 NULL, 56 DEFAULT]` |
| Bytea/binary cell renders NULL as the literal placeholder text **`NULL`** in the small format field, gray (`secondaryLabelColor`); with a value it shows the data format (`PNG` in the IB sample) | obj #321 `NSPlaceholderString = 'NULL'`, `NSContents = 'PNG'`, text color `secondaryLabelColor` (obj #122) |
| Text cell has **no** NULL/EMPTY literal in the nib — its NULL/EMPTY rendering is code-driven (likely same gray `NULL` placeholder / empty value display). The cell's `showMenu:` chevron menu is code-built (no menu object in the nib). | TextCell prototype strings: no `NULL`/`EMPTY`/`DEFAULT`; only `showMenu:` action |
| Adjacent (not proof): the main data-grid cell context menu in `TableDataController.nib` has a `Set Value ⌥↩` submenu with `EMPTY`, separator, `NULL`, `DEFAULT`, separator, `Add a file...` — likely the same vocabulary the row-detail chevron menus use. | `TableDataController.nib` `NSMenu` #303 items #305/#308/#310/#313 |
| Gray NULL is also visible in the workspace reference screenshot (`subtitulo_ficha` blob = NULL) | `reference_row_editor.webp` (non-static; labeled) |
| `"Convert NULL to EMPTY"` is **not** row detail — it is a CSV/JSON export option (`Export*Window.nib`); do not cite it for the editor. | grep over Resources nibs |

---

## 7. Cross-check against the workspace reference screenshots (non-static, for the Lead)

The workspace contains `reference_row_editor.webp`, `reference_row_editor_panel.png`, `reference_row_editor_top.png` (user-provided TablePlus screenshots, MySQL 8.0.33). They corroborate the nib evidence:

| Observed in reference | Matches nib evidence |
|---|---|
| Search field with magnifier at top of pane, placeholder `Search for field`, sliders/options button at right | `MenuSearchFieldCell` placeholder `Search for field...`; `NSSearchField` #127; `preferencesTemplate` button #153 |
| Per field: name at left, lowercase type at right, value box beneath with a downward chevron inside at the right edge | cell layout: 175 pt name + 110 pt type label, `RowDetailTextView` value, `AutoSizeButton` chevron; screenshot shows right-aligned types (nib center alignment differs) |
| NULL shown as gray `NULL` inside the value box | Bytea `NSPlaceholderString = 'NULL'`; gray `secondaryLabelColor` |
| Type labels `long`, `var_string`, `blob` | MySQL enum names lowercased (see §5) |
| Toolbar top-right has a 2-segment “panels” control (console/bottom + right sidebar icons), the right one matching `rightSidebarTemplate` | `NSSegmentedControl` #166 with segments #171 (`bottomSidebarTemplate`) / #177 (`rightSidebarTemplate`, tooltip `Toggle Row Detail (Space)`) |
| **No `Details` / `Assistant` header row is visible** in the screenshot | nib defines a 37 pt `buttonSegment` header with `Details`/`Assistant` (hidden in that screenshot). Inference: header/tabs only appear when the Assistant tab is enabled (setting “Enable Assistant Tab on the Right Sidebar”); otherwise the tab view/search fills the panel top. |
| Row pitch in the screenshot looks larger than the 37/44 pt nib prototypes (estimate; screenshot scale ~2×, pane was resized) | treat nib cell heights as minimum/design heights; real rows grow with content (text view height is not fixed; `RowDetailByteaCell` has a 184 pt image height constraint) |

---

## 8. What could NOT be determined statically

1. **Runtime widths/heights**: workspace pane width (240 constraint / 250 frame / 300 content design), actual default width, and runtime row heights (the row list height 230 is an IB design value; cell heights 37/44/230 are prototype sizes, not a per-row rule). The reference screenshot suggests the pane is user-resizable and rows are content-driven (estimates only).
2. **`rowLabel` runtime role/visibility**: `No row selected` text + frame are known, but the label has no constraints and the table scroll view is ordered above it. Whether it is a centered empty-state overlay, a footer, or shown only with certain state is code-driven (not in nib).
3. **Exact save timing inside the row detail** (end-editing staging vs immediate UPDATE). Evidence establishes the app-level explicit Commit/Discard/Preview workflow, but the row-detail delegate path is not in the nib; `RowDetailCellDelegate`/`RowDetailByteaCellDelegate`/`RowDetailDropdownCellDelegate` method names are Swift-only and not preserved.
4. **Text-cell NULL/EMPTY rendering** (only the Bytea placeholder and the Dropdown menu literals are statically visible; the text-cell chevron menu is built in code).
5. **Exact contents of code-built `showMenu:` menus** for text/bytea cells (only the adjacent main-grid “Set Value ⌥↩ → EMPTY/NULL/DEFAULT/Add a file…” menu is known).
6. **Field-type formatting algorithm** (lowercase? `COLUMN_TYPE`? per-driver mapping?) — only the driver enum-name lists and the rendered result (`long`/`var_string`/`blob` in the MySQL reference) are known.
7. **Space-key binding implementation** — only the tooltip `Toggle Row Detail (Space)`, the `"Space"` localizable key and `AppShortcuts`/`AppShortcutCommand` types; no nib menu item/key equivalent. Might be an app shortcut (Preferences → Shortcuts), not a hard-coded key.
8. **Visibility rules for the `Details`/`Assistant` header** (inference: hidden unless the Assistant tab is enabled; reference screenshot shows no header).
9. **Runtime enabled/disabled states** (many nib controls carry `NSEnabled = False`; that is an IB design state, not behavior).
10. **Localized text for the menu/tab labels** (English nib strings are the literal design strings; `Localizable.strings` may override some at runtime).
11. **Whether the panel is read-only for keyless tables/views** (only warning strings exist; no UI/flag in the nib).
12. **Assistant tab details** (chat UI, LLM consent, `copyAction:`/`deleteAction:`/`sendAction:`) — captured only as context; not row detail.

---

## 9. Implementation implications for a Rust/egui recreation

1. **Layout**: right `SidePanel::right` (resizable, min ≈ 240 pt, default ≈ 300 pt). Splitter with the same minimum; pane content width 300 at design time. [estimate for default: 300]
2. **Top of the pane**: search bar 24 pt high, 10 pt left inset, 4 pt gap below to the list; a 35×22 sliders/menu button at the right of the search row. Placeholder `Search for field` (TablePlus shows `Search for field...` in the nib; the reference screenshot shows `Search for field`).
3. **Optional header (behind Assistant setting)**: 37 pt strip with two 65×19 text buttons `Details` / `Assistant`, 10 pt apart, centered horizontally, y = 9.
4. **Field list**: one row per column, no selection highlight, scrollable, 6 pt bottom content inset. Row anatomy:
   - line 1 (≈17 pt): column name (11 pt, secondary/gray, left) · column type (11 pt, secondary/gray, right-aligned per reference; nib says center).
   - line 2: value editor, full width minus ~10 pt side margins; per-kind row heights from the nib prototypes: text 37, dropdown 44, bytea 230 (bytea includes 276×184 image preview). Make rows content-sized (estimates) since the reference shows multi-line rows and the text view height is dynamic.
5. **Per-field editors**:
   - text (`long`, `var_string`, `text`, `json`, …): multi-line text view for the value, plus a 20×18 down-arrow menu button at the right edge (12 pt from the right), `showMenu:` equivalent.
   - boolean/enum/dropdown: 280×24 popup at the left 10 pt with options `EMPTY`, `NULL`, `DEFAULT` and the actual values; `popupButtonDidSelect:` semantics.
   - blob/bytea: image preview area, small format/caption field (placeholder gray `NULL`), menu chevron button; TablePlus accepts image pasteboard types (PNG/TIFF/PDF/PICT/file names) — drop/paste support is plausible.
6. **NULL/EMPTY/DEFAULT**: show NULL as a gray literal `NULL` placeholder (not empty box); support explicit `EMPTY`/`NULL`/`DEFAULT` actions. Do not conflate with the export option “Convert NULL to EMPTY”.
7. **Edits / persistence**: TablePlus uses an explicit commit model (Commit ⌘S, Discard ⌘⇧⌫, Preview-as-SQL ⌘⇧P; pending-change highlighting). A faithful recreation should stage edits and commit explicitly; auto-save on focus-loss is a deliberate deviation (acceptable only if the acceptance criteria allow it).
8. **Selection sync**: row selection in the main data grid drives the detail list (the panel has no row-selection highlight of its own); selection changes reload all field values.
9. **Filter**: fuzzy (default) vs `Contains` matching over field names (menu items `Fuzzy - default` / `Contains`; `updateSearchStrategy:` + tag 0/1). Fuzzy default means subsequence/substring-like ranking; implement simple case-insensitive fuzzy with highlight as first pass. `>`-prefix “search in column content” hint is *not* confirmed for this field list.
10. **Toggle**: a 2-segment “panels” toolbar control (bottom panel + row detail), right segment toggles the pane, tooltip `Toggle Row Detail (Space)`; bind Space when no text field has keyboard focus (TablePlus documents Space only via tooltip/app shortcuts).
11. **Type label mapping** (Dataloom): map native column types to short lowercase tokens (`long`, `var_string`, `blob`, `text`, `integer`, `real`, …). For SQLite use the declared type lowercased; for MySQL map protocol type names (`LONGLONG`→`longlong`, `VAR_STRING`→`var_string`, `BLOB`/`TINY_BLOB`/`MEDIUM_BLOB`/`LONG_BLOB`→`blob`/`tiny_blob`/…); for PostgreSQL use `data_type` lowercased. Exact TablePlus formatting is not statically known — pick and document a mapping.
12. **Read-only**: disable inline editors and show a read-only indicator when the table/view has no usable primary key or the driver reports read-only (TablePlus strings: `Read-only`, `“%@” is read-only.`); guard updates against “affects multiple rows” (no-PK warning).
13. **Style tokens**: secondary label gray for names/types/NULL; control background for the pane; `gridRowDetailText`-equivalent theme color for row-detail text; chevron icon 14×14 inside 20×18 button.

---

## Appendix — reproducible extraction commands

```bash
# Nib class/outlet/action/frame evidence (objects/frames above)
python3 - <<'PY'   # requires: pip3 install nibarchive==1.1.0
from nibarchive import NIBArchiveParser
a = NIBArchiveParser().parse(open('/Applications/TablePlus.app/Contents/Resources/RightSidebarController.nib','rb'))
print(a.header)                    # object_count=691, key_count=237, value_count=3476, class_name_count=49
for i,o in enumerate(a.objects):
    print(i, a.get_class_name(o).name, [(k.name, v.data) for k,v in a.get_object_items(o).items()])
PY

# Swift/ObjC class names, selectors and property encodings (arm64 slice)
lipo -thin arm64 /Applications/TablePlus.app/Contents/MacOS/TablePlus -output /tmp/TablePlus_arm64
strings -a /tmp/TablePlus_arm64 | grep -E 'RowDetail|MenuSearchField|rightSidebar'
otool -v -s __TEXT __objc_methname /tmp/TablePlus_arm64 | grep -E 'toggleRightSidebar:|updateSearchStrategy:|popupButtonDidSelect:|showMenu:|commitChanges:'

# Localizable strings
plutil -p /Applications/TablePlus.app/Contents/Resources/en.lproj/Localizable.strings | grep -i 'row detail\|read-only\|search for field'

# Toolbar/asset evidence
strings -a /Applications/TablePlus.app/Contents/Resources/WorkspaceWindow.nib | grep 'Toggle Row Detail'
assetutil --info /Applications/TablePlus.app/Contents/Resources/Assets.car | grep -A1 rightSidebarTemplate
```
