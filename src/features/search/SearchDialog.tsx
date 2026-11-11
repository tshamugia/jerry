import {
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import type { DateFilter, FileType, SavedSearch, SearchItems, SizeRange } from "../../ipc";
import { closeOverlay, useOverlay } from "../../stores/overlay";
import { strings } from "../../strings";
import { Dialog } from "../../ui/Dialog";
import { DropdownItem, DropdownMenu } from "../../ui/DropdownMenu";
import { IconButton } from "../../ui/IconButton";
import { CheckIcon, CloseIcon, DropIcon, RecentIcon, SearchIcon } from "../../ui/icons";
import { Kbd } from "../../ui/Kbd";
import {
  DATE_PRESETS,
  dateField,
  dateLabel,
  EMPTY_SEARCH,
  filtersText,
  ITEMS,
  lastDay,
  rangeOf,
  SIZE_PRESETS,
  searchKey,
  sizeField,
  sizeLabel,
  sizePreset,
  sizeRangeOf,
  TYPE_GROUPS,
  typeLabel,
} from "./query";
import {
  forgetSearch,
  loadSearchHistory,
  runSearch,
  type SearchPlace,
  searchPlace,
  useSearchHistory,
} from "./search";

const t = strings.search;

/** A filter button's look: a 28 px control with its value and an arrow. */
const FILTER =
  "flex h-filter min-w-0 items-center gap-1.5 rounded-ctrl border border-ctrl-border bg-ctrl pr-1.5 pl-2.5 text-small hover:bg-hover data-[state=open]:bg-hover";
const FIELD =
  "h-filter w-32 min-w-0 rounded-ctrl border border-ctrl-border bg-ctrl px-2 text-small outline-none placeholder:text-fg-3 focus:outline-2 focus:outline-focus focus:-outline-offset-2";

/**
 * The search window (Ctrl+Shift+F, PRD «ღრმა ძებნა»): the name to look for (parts of it, `*` and
 * `?`, several words), where (the folder with its subfolders, or its whole drive), and filters by
 * type, size, date modified and kind. Recent searches drop below the field and narrow as the user
 * types: ↑↓ pick one, Enter runs it, Delete forgets it. Enter otherwise searches as the form says;
 * the results open in a tab of their own, or in place when the window edits a results tab's search.
 */
export function SearchDialog() {
  const open = useOverlay((state) => state.overlay === "search");
  const input = useRef<HTMLInputElement>(null);
  return (
    <Dialog
      open={open}
      onClose={closeOverlay}
      title={t.title}
      closeHint={t.close}
      initialFocus={input}
    >
      {open && <SearchForm input={input} />}
    </Dialog>
  );
}

/** The fields a custom filter opens; undefined while its menu has another choice. */
interface Fields {
  extension: string | undefined;
  size: { min: string; max: string } | undefined;
  dates: { from: string; to: string } | undefined;
}

function fieldsOf(search: SavedSearch): Fields {
  const { fileType, size, modified } = search;
  return {
    extension: fileType?.kind === "extension" ? fileType.extension : undefined,
    size:
      size !== null && sizePreset(size) === undefined
        ? {
            min: size.min === null ? "" : sizeField(size.min),
            max: size.max === null ? "" : sizeField(size.max),
          }
        : undefined,
    dates:
      modified?.kind === "range"
        ? {
            from: modified.from === null ? "" : dateField(modified.from),
            to: modified.to === null ? "" : dateField(lastDay(modified.to)),
          }
        : undefined,
  };
}

/** The search the form says, or why it can't be run. */
function compose(form: SavedSearch, fields: Fields): SavedSearch | { problem: string } {
  const fileType: FileType | null =
    fields.extension === undefined
      ? form.fileType
      : { kind: "extension", extension: fields.extension.trim() };
  let size = form.size;
  if (fields.size) {
    const range = sizeRangeOf(fields.size.min, fields.size.max);
    if (range === undefined) {
      return { problem: t.badSize };
    }
    size = range.min === null && range.max === null ? null : range;
  }
  let modified = form.modified;
  if (fields.dates) {
    const range = rangeOf(fields.dates.from, fields.dates.to);
    if (range === undefined || range.kind !== "range") {
      return { problem: t.badDate };
    }
    modified = range.from === null && range.to === null ? null : range;
  }
  return { ...form, name: form.name.trim(), fileType, size, modified };
}

function SearchForm({ input }: { input: RefObject<HTMLInputElement | null> }) {
  // The active pane can't change while the window is open.
  const place = useMemo(() => searchPlace(), []);
  const start = place?.editing ?? EMPTY_SEARCH;
  const [form, setForm] = useState<SavedSearch>(start);
  const [fields, setFields] = useState<Fields>(() => fieldsOf(start));
  const [highlight, setHighlight] = useState<number>();
  const [tried, setTried] = useState(false);
  const history = useSearchHistory((state) => state.searches);
  const listId = useId();

  useEffect(() => {
    void loadSearchHistory();
  }, []);

  const composed = compose(form, fields);
  const problem = "problem" in composed ? composed.problem : undefined;
  const recent = useMemo(() => {
    const typed = form.name.trim().toLowerCase();
    return (history ?? []).filter((search) => search.name.toLowerCase().includes(typed));
  }, [history, form.name]);
  const picked = highlight === undefined ? undefined : recent[highlight];

  const update = (change: Partial<SavedSearch>) =>
    setForm((current) => ({ ...current, ...change }));
  const updateFields = (change: Partial<Fields>) =>
    setFields((current) => ({ ...current, ...change }));

  const run = (search: SavedSearch) => {
    if (place) runSearch(search, place);
  };
  const search = () => {
    if (picked) {
      run(picked);
      return;
    }
    setTried(true);
    if (!("problem" in composed)) {
      run(composed);
    }
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    search();
  };
  // Enter in any of the form's fields searches; in the name field, ↑↓ and Delete work the recent list.
  const keyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) {
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      search();
    } else if (recent.length === 0) {
      return;
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      setHighlight((at) => (at === undefined ? 0 : Math.min(at + 1, recent.length - 1)));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setHighlight((at) => (at === undefined || at === 0 ? undefined : at - 1));
    } else if (event.key === "Delete" && picked) {
      event.preventDefault();
      forgetSearch(picked);
      setHighlight((at) =>
        at === undefined || recent.length <= 1
          ? undefined
          : Math.max(0, Math.min(at, recent.length - 2)),
      );
    }
  };

  return (
    <form onSubmit={submit} className="flex flex-col">
      <label className="flex h-13 items-center gap-2.5 border-stroke border-b px-4 text-fg-2">
        <SearchIcon />
        <input
          ref={input}
          type="text"
          value={form.name}
          onChange={(event) => {
            update({ name: event.target.value });
            setHighlight(undefined);
          }}
          onKeyDown={keyDown}
          onFocus={(event) => event.currentTarget.select()}
          aria-label={t.name}
          aria-autocomplete="list"
          aria-controls={recent.length > 0 ? listId : undefined}
          aria-activedescendant={picked ? `${listId}-${highlight}` : undefined}
          placeholder={t.placeholder}
          spellCheck={false}
          autoComplete="off"
          className="min-w-0 flex-1 bg-transparent text-fg text-input outline-none placeholder:text-fg-3"
        />
      </label>
      <div className="flex flex-col gap-3 px-4 py-3">
        {place && <Scope place={place} wholeDrive={form.wholeDrive} onChange={update} />}
        <Row label={t.filters}>
          <TypeMenu
            value={form.fileType}
            extension={fields.extension !== undefined}
            onPick={(fileType) => {
              update({ fileType });
              updateFields({ extension: undefined });
            }}
            onExtension={() => updateFields({ extension: fields.extension ?? "" })}
          />
          <SizeMenu
            value={form.size}
            custom={fields.size !== undefined}
            onPick={(size) => {
              update({ size });
              updateFields({ size: undefined });
            }}
            onCustom={() => updateFields({ size: fields.size ?? { min: "", max: "" } })}
          />
          <DateMenu
            value={form.modified}
            range={fields.dates !== undefined}
            onPick={(modified) => {
              update({ modified });
              updateFields({ dates: undefined });
            }}
            onRange={() => updateFields({ dates: fields.dates ?? { from: "", to: "" } })}
          />
          <ItemsMenu value={form.items} onPick={(items) => update({ items })} />
        </Row>
        <CustomFields fields={fields} onChange={updateFields} onEnter={search} />
        {problem && (tried || fieldsTouched(fields)) && (
          <p role="alert" className="text-red text-small">
            {problem}
          </p>
        )}
      </div>
      {recent.length > 0 && (
        <Recent
          id={listId}
          searches={recent}
          highlight={highlight}
          onRun={run}
          onForget={(search) => {
            forgetSearch(search);
            setHighlight(undefined);
            input.current?.focus();
          }}
        />
      )}
      <footer className="flex h-9 shrink-0 items-center gap-4 border-stroke border-t px-4 text-caption text-fg-3">
        <span className="flex items-center gap-1.5">
          <Kbd>↵</Kbd> {t.hints.search}
        </span>
        {recent.length > 0 && (
          <span className="flex items-center gap-1.5">
            <Kbd>↑↓</Kbd> {t.hints.recent}
          </span>
        )}
        <span className="flex items-center gap-1.5">
          <Kbd>Esc</Kbd> {t.hints.close}
        </span>
      </footer>
    </form>
  );
}

/** Whether a custom field has text: its problem shows as the user types, not before. */
function fieldsTouched(fields: Fields): boolean {
  return (
    (fields.size !== undefined && (fields.size.min !== "" || fields.size.max !== "")) ||
    (fields.dates !== undefined && (fields.dates.from !== "" || fields.dates.to !== ""))
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-3">
      <span className="w-20 shrink-0 text-fg-2 text-small">{label}</span>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">{children}</div>
    </div>
  );
}

/** The folder with its subfolders, or its whole drive. */
function Scope({
  place,
  wholeDrive,
  onChange,
}: {
  place: SearchPlace;
  wholeDrive: boolean;
  onChange: (change: Partial<SavedSearch>) => void;
}) {
  const option = (drive: boolean, label: string) => (
    <button
      type="button"
      aria-pressed={wholeDrive === drive}
      onClick={() => onChange({ wholeDrive: drive })}
      className="h-6 min-w-0 truncate rounded-kbd px-2.5 text-small hover:bg-hover aria-pressed:bg-nav-sel aria-pressed:text-accent-text"
    >
      {label}
    </button>
  );
  return (
    <Row label={t.scope}>
      <fieldset
        aria-label={t.scope}
        className="flex min-w-0 gap-0.5 rounded-ctrl border border-ctrl-border bg-ctrl p-0.5"
      >
        {option(false, t.thisFolder(place.name))}
        {option(true, t.wholeDriveOf(place.drive))}
      </fieldset>
    </Row>
  );
}

/** A menu item with a check mark on the chosen one. */
function Choice({
  checked,
  onSelect,
  children,
}: {
  checked: boolean;
  onSelect: () => void;
  children: ReactNode;
}) {
  return (
    <DropdownItem onSelect={onSelect}>
      <span className="flex size-4 shrink-0 items-center justify-center text-accent-text">
        {checked && <CheckIcon />}
      </span>
      <span className="truncate">{children}</span>
    </DropdownItem>
  );
}

function Menu({ label, value, children }: { label: string; value: string; children: ReactNode }) {
  return (
    <DropdownMenu
      label={label}
      trigger={
        <button type="button" aria-label={`${label}: ${value}`} className={FILTER}>
          <span className="truncate">{value}</span>
          <span className="text-fg-3">
            <DropIcon />
          </span>
        </button>
      }
    >
      {children}
    </DropdownMenu>
  );
}

function TypeMenu({
  value,
  extension,
  onPick,
  onExtension,
}: {
  value: FileType | null;
  extension: boolean;
  onPick: (type: FileType | null) => void;
  onExtension: () => void;
}) {
  const shown = extension ? t.extensionField : typeLabel(value);
  return (
    <Menu label={t.type} value={shown}>
      <Choice checked={!extension && value === null} onSelect={() => onPick(null)}>
        {t.anyType}
      </Choice>
      {TYPE_GROUPS.map((kind) => (
        <Choice
          key={kind}
          checked={!extension && value?.kind === kind}
          onSelect={() => onPick({ kind })}
        >
          {t.types[kind]}
        </Choice>
      ))}
      <Choice checked={extension} onSelect={onExtension}>
        {t.extension}
      </Choice>
    </Menu>
  );
}

function SizeMenu({
  value,
  custom,
  onPick,
  onCustom,
}: {
  value: SizeRange | null;
  custom: boolean;
  onPick: (size: SizeRange | null) => void;
  onCustom: () => void;
}) {
  const shown = custom ? t.customSizeLabel : sizeLabel(value);
  const preset = sizePreset(value);
  return (
    <Menu label={t.size} value={shown}>
      <Choice checked={!custom && value === null} onSelect={() => onPick(null)}>
        {t.anySize}
      </Choice>
      {SIZE_PRESETS.map(({ label, size }) => (
        <Choice key={label} checked={!custom && preset === label} onSelect={() => onPick(size)}>
          {label}
        </Choice>
      ))}
      <Choice checked={custom} onSelect={onCustom}>
        {t.customSize}
      </Choice>
    </Menu>
  );
}

function DateMenu({
  value,
  range,
  onPick,
  onRange,
}: {
  value: DateFilter | null;
  range: boolean;
  onPick: (modified: DateFilter | null) => void;
  onRange: () => void;
}) {
  const shown = range ? t.dateRangeLabel : dateLabel(value, dateField);
  return (
    <Menu label={t.modified} value={shown}>
      <Choice checked={!range && value === null} onSelect={() => onPick(null)}>
        {t.anyTime}
      </Choice>
      {DATE_PRESETS.map((kind) => (
        <Choice
          key={kind}
          checked={!range && value?.kind === kind}
          onSelect={() => onPick({ kind })}
        >
          {t.dates[kind]}
        </Choice>
      ))}
      <Choice checked={range} onSelect={onRange}>
        {t.dateRange}
      </Choice>
    </Menu>
  );
}

function ItemsMenu({
  value,
  onPick,
}: {
  value: SearchItems;
  onPick: (items: SearchItems) => void;
}) {
  return (
    <Menu label={t.kind} value={t.items[value]}>
      {ITEMS.map((items) => (
        <Choice key={items} checked={value === items} onSelect={() => onPick(items)}>
          {t.items[items]}
        </Choice>
      ))}
    </Menu>
  );
}

/** The fields a custom filter opened: an extension, a size range, a date range. */
function CustomFields({
  fields,
  onChange,
  onEnter,
}: {
  fields: Fields;
  onChange: (change: Partial<Fields>) => void;
  onEnter: () => void;
}) {
  const { extension, size, dates } = fields;
  if (extension === undefined && size === undefined && dates === undefined) {
    return null;
  }
  const field = (
    label: string,
    value: string,
    placeholder: string,
    change: (value: string) => void,
  ) => (
    <label className="flex items-center gap-2 text-fg-2 text-small">
      {label}
      <input
        type="text"
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        autoComplete="off"
        // A field opened from its menu takes the keyboard.
        // biome-ignore lint/a11y/noAutofocus: it opens because the user asked for it.
        autoFocus={value === ""}
        onChange={(event) => change(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.nativeEvent.isComposing) {
            event.preventDefault();
            onEnter();
          }
        }}
        className={`${FIELD} text-fg`}
      />
    </label>
  );
  return (
    <Row label="">
      {extension !== undefined &&
        field(t.extensionField, extension, t.extensionPlaceholder, (value) =>
          onChange({ extension: value }),
        )}
      {size !== undefined && (
        <>
          {field(t.atLeast, size.min, t.sizePlaceholder, (min) =>
            onChange({ size: { ...size, min } }),
          )}
          {field(t.atMost, size.max, t.sizePlaceholder, (max) =>
            onChange({ size: { ...size, max } }),
          )}
        </>
      )}
      {dates !== undefined && (
        <>
          {field(t.from, dates.from, t.datePlaceholder, (from) =>
            onChange({ dates: { ...dates, from } }),
          )}
          {field(t.to, dates.to, t.datePlaceholder, (to) => onChange({ dates: { ...dates, to } }))}
        </>
      )}
    </Row>
  );
}

/** Recent searches, newest first, narrowed by the typed name. */
function Recent({
  id,
  searches,
  highlight,
  onRun,
  onForget,
}: {
  id: string;
  searches: readonly SavedSearch[];
  highlight: number | undefined;
  onRun: (search: SavedSearch) => void;
  onForget: (search: SavedSearch) => void;
}) {
  return (
    <div className="flex flex-col border-stroke border-t pb-1.5">
      <p className="px-4 pt-2.5 pb-1 text-caption text-fg-3">{t.recent}</p>
      <div
        id={id}
        role="listbox"
        aria-label={t.recent}
        // Picking with the mouse keeps the field's focus.
        onMouseDown={(event) => event.preventDefault()}
        className="flex flex-col px-1.5"
      >
        {searches.map((search, at) => {
          const filters = filtersText(search, dateField);
          const picked = at === highlight;
          return (
            // biome-ignore lint/a11y/useKeyWithClickEvents: the field's ↑↓ and Enter pick it (aria-activedescendant).
            <div
              key={searchKey(search)}
              id={`${id}-${at}`}
              role="option"
              tabIndex={-1}
              aria-selected={picked}
              onClick={() => onRun(search)}
              className={`group relative flex h-palette-row cursor-default items-center gap-2.5 rounded-ctrl pr-1 pl-2.5 ${picked ? "bg-sel" : "hover:bg-hover"}`}
            >
              {picked && (
                <span className="absolute top-1/2 left-0 h-4 w-0.75 -translate-y-1/2 rounded-bar bg-accent" />
              )}
              <span className="text-fg-3">
                <RecentIcon />
              </span>
              <span className="min-w-0 truncate font-semibold">
                {search.name === "" ? t.anyName : search.name}
              </span>
              {filters !== "" && (
                <span className="min-w-0 flex-1 truncate text-fg-3 text-small">{filters}</span>
              )}
              <span className="flex-1" />
              <IconButton
                label={t.removeRecent}
                tabIndex={-1}
                onClick={(event) => {
                  event.stopPropagation();
                  onForget(search);
                }}
                className={`size-6 rounded-kbd text-fg-2 ${picked ? "" : "invisible group-hover:visible"}`}
              >
                <CloseIcon />
              </IconButton>
            </div>
          );
        })}
      </div>
    </div>
  );
}
