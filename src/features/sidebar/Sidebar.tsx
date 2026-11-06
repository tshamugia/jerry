import { memo, useEffect, useRef, useState } from "react";
import { commandById, isEnabled, runCommand } from "../../commands/registry";
import { type Drive, driveIconUrl, type NetworkPlace, type Place, placeIconUrl } from "../../ipc";
import { useCommandLabel } from "../../stores/keymap";
import { errorText, sizeText, strings } from "../../strings";
import { ContextMenu, MenuItem } from "../../ui/ContextMenu";
import { Dialog } from "../../ui/Dialog";
import { IconButton } from "../../ui/IconButton";
import { EjectIcon, PlusIcon, SettingsIcon, SidebarIcon } from "../../ui/icons";
import {
  FavoriteExplained,
  FavoritesFailed,
  FavoritesSection,
  RecentSection,
} from "./FavoritesSection";
import {
  BUTTON,
  Caption,
  folderKey,
  ITEM,
  ITEM_LOOK,
  ItemIcon,
  moveFocus,
  NavItem,
  openHandlers,
  Pill,
  Separator,
  useCurrentPath,
} from "./items";
import {
  addNetworkPlace,
  dismissEjectFailed,
  driveRoot,
  eject,
  FULL,
  openPlace,
  openRecycleBin,
  removeNetworkPlace,
  splitDrives,
  startAdding,
  stopAdding,
  toggleSidebar,
  usedShare,
  useSidebar,
} from "./store";

const t = strings.sidebar;

/**
 * The sidebar, as the design draws it on Mica: the top row (hide the sidebar, settings),
 * Favorites with their groups, Recent, the standard places, Drives with their free space,
 * Network (mapped drives and the locations the user added) and Recycle Bin. A click opens the
 * place in the active pane's tab, Ctrl+click or a middle click in a new tab; the item of the
 * folder the active tab shows is selected. Up and Down move between the items.
 */
export const Sidebar = memo(function Sidebar() {
  const places = useSidebar((state) => state.places);
  const drives = useSidebar((state) => state.drives.drives);
  const network = useSidebar((state) => state.network);
  const folded = useSidebar((state) => state.folded);
  const adding = useSidebar((state) => state.adding);
  const current = folderKey(useCurrentPath());
  const sidebarLabel = useCommandLabel("toggleSidebar");
  const settingsLabel = useCommandLabel("settings");
  const settings = commandById("settings");
  const { local, network: mapped } = splitDrives(drives);
  const selected = (path: string) => current !== undefined && folderKey(path) === current;

  return (
    <nav aria-label={strings.frame.sidebar} className="flex min-h-0 min-w-0 flex-col">
      <div className="flex h-sidebar-header shrink-0 items-center justify-between px-2.5">
        <IconButton label={sidebarLabel} onClick={toggleSidebar} className="size-8 rounded-ctrl">
          <SidebarIcon />
        </IconButton>
        <IconButton
          label={settingsLabel}
          disabled={!settings || !isEnabled(settings)}
          onClick={() => settings && runCommand(settings)}
          className="size-8 rounded-ctrl"
        >
          <SettingsIcon />
        </IconButton>
      </div>
      <div data-nav-items className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pb-2">
        <FavoritesSection current={current} />
        <RecentSection />
        <Separator />
        {places.map((place) => (
          <NavItem
            key={place.place}
            path={place.path}
            icon={<ItemIcon src={placeIconUrl(place.place, 16)} />}
            label={place.name ?? t.places[place.place]}
            selected={selected(place.path)}
          />
        ))}
        <Separator />
        <Caption section="drives" label={t.drives} folded={folded.has("drives")} />
        {!folded.has("drives") &&
          local.map((drive) => (
            <DriveItem
              key={drive.letter}
              drive={drive}
              selected={selected(driveRoot(drive.letter))}
            />
          ))}
        <Separator />
        <Caption
          section="network"
          label={t.network}
          folded={folded.has("network")}
          action={
            <IconButton
              label={t.addNetworkPlace}
              onClick={startAdding}
              className="size-6 rounded-ctrl text-fg-2"
            >
              <PlusIcon />
            </IconButton>
          }
        />
        {!folded.has("network") && (
          <>
            {mapped.map((drive) => (
              <NetworkDriveItem
                key={drive.letter}
                drive={drive}
                selected={selected(driveRoot(drive.letter))}
              />
            ))}
            {network.map((place) => (
              <NetworkPlaceItem key={place.path} place={place} selected={selected(place.path)} />
            ))}
            {adding && <NetworkField />}
          </>
        )}
        <RecycleBinItem />
      </div>
      <EjectFailed />
      <FavoriteExplained />
      <FavoritesFailed />
    </nav>
  );
});

function driveName(drive: Drive): string {
  return t.driveName(drive.letter, drive.kind, drive.label);
}

/** A local drive: name, free space, and the usage bar, red at 90 % used; Eject when it ejects. */
function DriveItem({ drive, selected }: { drive: Drive; selected: boolean }) {
  const name = driveName(drive);
  const used = usedShare(drive);
  const full = used !== undefined && used >= FULL;
  const space = drive.space;
  const root = driveRoot(drive.letter);
  return (
    <div
      className={`relative flex shrink-0 items-center rounded-ctrl ${selected ? "bg-nav-sel" : ""}`}
    >
      {selected && <Pill />}
      <button
        type="button"
        {...{ [ITEM]: "" }}
        title={space ? t.driveDetails(name, sizeText(space.free), sizeText(space.total)) : root}
        aria-current={selected ? "location" : undefined}
        className={`${ITEM_LOOK} items-start px-2 py-drive-pad`}
        {...openHandlers((newTab) => openPlace(root, newTab))}
      >
        <ItemIcon src={driveIconUrl(drive.letter, 16)} />
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="flex items-baseline justify-between gap-2">
            <span className="truncate">{name}</span>
            {space && (
              <span className={`shrink-0 text-caption ${full ? "text-red" : "text-fg-3"}`}>
                {t.free(sizeText(space.free))}
              </span>
            )}
          </span>
          {used !== undefined && (
            <span className="h-1 overflow-hidden rounded-bar bg-track">
              <span
                className={`block h-1 rounded-bar ${full ? "bg-red" : "bg-accent"}`}
                style={{ width: `${Math.round(used * 100)}%` }}
              />
            </span>
          )}
        </span>
      </button>
      {drive.ejectable && (
        <IconButton
          label={t.eject(name)}
          onClick={() => void eject(drive.letter)}
          className="mr-0.5 size-6 rounded-ctrl text-fg-2"
        >
          <EjectIcon />
        </IconButton>
      )}
    </div>
  );
}

/** A mapped network drive: its share's name and letter, its server at the end. */
function NetworkDriveItem({ drive, selected }: { drive: Drive; selected: boolean }) {
  const name = driveName(drive);
  const server = drive.remote?.replace(/^\\\\/, "").split("\\")[0];
  return (
    <NavItem
      path={driveRoot(drive.letter)}
      icon={<ItemIcon src={driveIconUrl(drive.letter, 16)} />}
      label={name}
      detail={server}
      title={drive.connected ? (drive.remote ?? undefined) : t.disconnected(drive.remote ?? name)}
      selected={selected}
    />
  );
}

/** A network location the user added; its right-click menu removes it. */
function NetworkPlaceItem({ place, selected }: { place: NetworkPlace; selected: boolean }) {
  return (
    <ContextMenu
      label={t.placeMenu}
      menu={
        <>
          <MenuItem onSelect={() => openPlace(place.path, true)}>{t.openInNewTab}</MenuItem>
          <MenuItem onSelect={() => removeNetworkPlace(place.path)}>{t.removeFromSidebar}</MenuItem>
        </>
      }
    >
      <NavItem
        path={place.path}
        icon={<ItemIcon src={placeIconUrl("share" satisfies Place, 16)} />}
        label={place.name}
        detail={place.server}
        selected={selected}
      />
    </ContextMenu>
  );
}

/** Types a network location to add: Enter adds it, Esc (or leaving it empty) cancels. */
function NetworkField() {
  const input = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState("");
  const [problem, setProblem] = useState<string>();
  useEffect(() => input.current?.focus(), []);

  const add = async () => {
    const error = await addNetworkPlace(value);
    if (error) {
      setProblem(error.code === "InvalidName" ? t.notAShare : errorText(error));
    }
  };
  return (
    <div className="flex shrink-0 flex-col gap-1 px-1 py-1">
      <input
        ref={input}
        value={value}
        aria-label={t.networkField}
        aria-invalid={problem !== undefined}
        aria-describedby="network-field-hint"
        placeholder={t.networkPlaceholder}
        spellCheck={false}
        onChange={(event) => {
          setValue(event.target.value);
          setProblem(undefined);
        }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) {
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            void add();
          } else if (event.key === "Escape") {
            event.preventDefault();
            stopAdding();
          }
        }}
        onBlur={() => value.trim() === "" && stopAdding()}
        className="h-filter min-w-0 rounded-ctrl border border-ctrl-border bg-ctrl px-2 text-fg text-small placeholder:text-fg-3"
      />
      <span
        id="network-field-hint"
        className={`px-1 text-caption ${problem ? "text-red" : "text-fg-3"}`}
      >
        {problem ?? t.networkHint}
      </span>
    </div>
  );
}

/** Recycle Bin opens in Explorer until Jerry's own view (v1.0). Its icon shows if it is full. */
function RecycleBinItem() {
  // The icon is asked for again when the window comes back to the front: it may have filled.
  const [look, setLook] = useState(0);
  useEffect(() => {
    const again = () => setLook((n) => n + 1);
    window.addEventListener("focus", again);
    return () => window.removeEventListener("focus", again);
  }, []);
  return (
    <button
      type="button"
      {...{ [ITEM]: "" }}
      className={`${ITEM_LOOK} h-nav shrink-0 px-2`}
      onClick={openRecycleBin}
      onKeyDown={moveFocus}
    >
      <ItemIcon src={`${placeIconUrl("recyclebin", 16)}&look=${look}`} />
      <span className="min-w-0 flex-1 truncate">{t.recycleBin}</span>
    </button>
  );
}

/** Jerry couldn't eject a drive (Windows explains a drive in use itself). */
function EjectFailed() {
  const failed = useSidebar((state) => state.ejectFailed);
  return (
    <Dialog
      open={failed !== undefined}
      onClose={dismissEjectFailed}
      title={t.ejectFailed}
      closeHint={t.closeHint}
    >
      <p className="break-words px-4 pt-3 text-fg">{failed && errorText(failed.error)}</p>
      <div className="flex justify-end gap-2 p-4">
        <button
          type="button"
          onClick={() => failed && void eject(failed.letter)}
          className={BUTTON}
        >
          {t.retry}
        </button>
        <button type="button" onClick={dismissEjectFailed} className={BUTTON}>
          {t.close}
        </button>
      </div>
    </Dialog>
  );
}
