# Changelog

What changed in each Jerry release. Generated from the commit history by [git-cliff](https://git-cliff.org).

## [0.1.0-rc.2](https://github.com/tshamugia/jerry/releases/tag/v0.1.0-rc.2) (2026-11-08)

### Features

- Core foundation with JerryError, IPC wrappers and logging ([d6354a2](https://github.com/tshamugia/jerry/commit/d6354a23f415f85ccaf72cefaaac41b774e791c7))
- SQLite storage, migrations and settings ([de3d403](https://github.com/tshamugia/jerry/commit/de3d40340f44b96ab7b82f7b0d6a82a016285310))
- Stream folder listings over a binary channel ([696b548](https://github.com/tshamugia/jerry/commit/696b548761e3235f23cf9425501e4f23fa7085e1))
- Natural sort in Rust with binary orders ([afd374e](https://github.com/tshamugia/jerry/commit/afd374ed64be4907e29a55851aff14d2bbefdbce))
- Window frame with zones, theme, density, accent and Mica ([d8e155d](https://github.com/tshamugia/jerry/commit/d8e155dd2f11b71dc990294365da631796fbd7c2))
- Live listings through a ReadDirectoryChangesW watcher ([203af09](https://github.com/tshamugia/jerry/commit/203af096be93c7e487965c8c97571b01cb2a6c40))
- Icons through a Shell pool and the icon:// protocol ([c98ed94](https://github.com/tshamugia/jerry/commit/c98ed9405d44e42cfcc0a13d5a3073b4a760069b))
- Icon keys in listing batches and icons in the dev listing ([43e85b1](https://github.com/tshamugia/jerry/commit/43e85b15d1b2731c215152b2b1f0783b0d23286b))
- Title bar with split tab strip and native caption buttons ([8945a8d](https://github.com/tshamugia/jerry/commit/8945a8ddaac417c8ed11e3d33405b9043779dc28))
- Command registry and keymap with the F1 shortcut table ([9e69006](https://github.com/tshamugia/jerry/commit/9e6900662e73cec715543904915cefbc8b5d70c0))
- Details view file list ([995cce0](https://github.com/tshamugia/jerry/commit/995cce09f2f3cabc05b6858c76212dabb242090b))
- Selection and keyboard movement in the file list ([7042575](https://github.com/tshamugia/jerry/commit/7042575b9125b588c9a24b97477da38e240c417d))
- Time select all in the dev panel ([1759d1e](https://github.com/tshamugia/jerry/commit/1759d1ed9b698530f2c9b26df5e03b5ade7ff54e))
- Quick filter (Ctrl+F) ([0e006c4](https://github.com/tshamugia/jerry/commit/0e006c4edcef3c0092116b5731fdb5cc6a677033))
- Select by pattern (Ctrl+Shift+A) with the quick filter's wildcards ([888844b](https://github.com/tshamugia/jerry/commit/888844bb3ac86c93e4fa76ab7f5633beffe87408))
- Tabs and navigation ([34402c7](https://github.com/tshamugia/jerry/commit/34402c75e9fbb7660cfb4024a272d456cbef4ca3))
- Dual-pane mode ([d456de1](https://github.com/tshamugia/jerry/commit/d456de11d47c193bb8626ba9d72a012d0d2d5dc9))
- **core:** Permanent delete that takes long paths and never follows links ([f8198dd](https://github.com/tshamugia/jerry/commit/f8198dd7379707305eb9a479da6297c421cfed84))
- **win:** Recycle Bin delete that never deletes for good on its own ([18828c5](https://github.com/tshamugia/jerry/commit/18828c5fd4e240f209756c6d2b737190db55c927))
- Delete commands for entries of an open listing ([b5ee296](https://github.com/tshamugia/jerry/commit/b5ee29618a6a7067964038cd0f6b148adffe4f33))
- Del and Shift+Del with confirmations ([8ca9523](https://github.com/tshamugia/jerry/commit/8ca95234dab5b6ef82c54ec95a9decabe5475e68))
- What the address bar's text names, completions and subfolders ([2e18bc9](https://github.com/tshamugia/jerry/commit/2e18bc9e88c36d4134c3d35a3165d1c86b9c1973))
- Address bar with segments, folder lists and text mode ([11c88ea](https://github.com/tshamugia/jerry/commit/11c88ea0d9f774f45453b8a247aef67e4d466e08))
- Name rules, rename and new folder in the core ([2a3de1c](https://github.com/tshamugia/jerry/commit/2a3de1c771577f531c7eb41daa6dee4b0a7e16a2))
- New folder and inline rename in the file list ([05ec203](https://github.com/tshamugia/jerry/commit/05ec203b377b0061edd1f1def2695e8fc326f0a8))
- Session restore and per-folder views ([dc92a14](https://github.com/tshamugia/jerry/commit/dc92a144f4bb4d8722a8e246a8e998de78813fe5))
- Drives, places and network locations for the sidebar ([681a096](https://github.com/tshamugia/jerry/commit/681a0961d234655af48deb3e8fe65edcf1ee0140))
- Sidebar with places, drives and network locations ([ab0d539](https://github.com/tshamugia/jerry/commit/ab0d539aa6780c2c0b05d0fe5dbd702c7d313747))
- Copy, cut and paste files with Explorer through the clipboard ([ab0be5a](https://github.com/tshamugia/jerry/commit/ab0be5a08bd81d054bc33da954618fdfb748a310))
- Sidebar favorites, groups and recent folders ([014da93](https://github.com/tshamugia/jerry/commit/014da9363ed068200c0745cd3cb75aec6549976d))
- Status bar with item count, selection size and free space ([afafcc5](https://github.com/tshamugia/jerry/commit/afafcc583ac5ed5727fa92fa8e4e43e93fc6eb56))
- Single instance, start arguments and running in the background ([5de163d](https://github.com/tshamugia/jerry/commit/5de163d2ad32615053bb14882846d18b08e85b8a))
- The file list's context menu, Show in Explorer and Open in Terminal ([54b3fcd](https://github.com/tshamugia/jerry/commit/54b3fcdb15758759dc10377985dde6034d43b7c3))
- Update checks, the update notice and signed release packages ([38f9770](https://github.com/tshamugia/jerry/commit/38f9770448462a36554d701a33960dfad33ff212))

### Fixes

- Match prestarted listings without waiting on a change being applied ([bbc8052](https://github.com/tshamugia/jerry/commit/bbc80528473bdb9bb450019eea820f29e978505f))
- Set up the system image lists once before icon lookups ([f579dca](https://github.com/tshamugia/jerry/commit/f579dca2e2ad13481f090f4b502e1462147864e3))
- Keep Shell pool workers from becoming COM's main STA ([d9181c4](https://github.com/tshamugia/jerry/commit/d9181c46f815b6d18e6fc1f287e57b3274612cfe))
- Release a volume's handles when Windows wants to remove it ([6bb0a96](https://github.com/tshamugia/jerry/commit/6bb0a96c8235c28dbef11407035f5e67ea549404))
- Dialogs above the list's sticky column header; selection in the Tech PRD ([779a6c4](https://github.com/tshamugia/jerry/commit/779a6c43be7343e5bdd3974f44fc2c9dc5510e23))
- Capture the pointer only once a marquee starts ([ee95e03](https://github.com/tshamugia/jerry/commit/ee95e0354b6e745c6bb50ee9c529a30bd9981e20))
- **win:** Hold queued Shell items so callbacks map to the right path ([9e65be3](https://github.com/tshamugia/jerry/commit/9e65be307910758d8c19913b75e9abb4fccebd53))
- Fold the address bar's segments again for every folder ([7b8b7c8](https://github.com/tshamugia/jerry/commit/7b8b7c888e14d86662b61eb673ef366cff5b2427))
- Give the focus back when a dialog closes ([7bbc2a2](https://github.com/tshamugia/jerry/commit/7bbc2a2927d9128fd1c08b660d6a2f83f3f3fd2e))
- A restored tab falls back to its own view if its folder's can't be read ([f68c5dc](https://github.com/tshamugia/jerry/commit/f68c5dc7d987c5d8d6579509fb2e744fb27f1532))
- Menu items that open a name field keep the focus ([48ae045](https://github.com/tshamugia/jerry/commit/48ae04563b191353b37890b90ab7e4f6bc5f0872))
- Visible menu highlight in dark mode, no browser menu over Jerry's ([4b822da](https://github.com/tshamugia/jerry/commit/4b822dac5a9bfd74748319ecdb06ab26dfe31f0a))

### Performance

- Keep database writes off the start and the window's settings off the storage lock ([46247b4](https://github.com/tshamugia/jerry/commit/46247b41cda86efe95b92e6468ab81965e4d2a0f))


