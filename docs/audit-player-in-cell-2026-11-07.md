# Audit: from command 64's stop to the player in the prison, 2026-11-07

Read-only research (branch `gameplay-loop`, nothing edited but this file). Sources: `docs/open-items.md`, the port's `crates/recomp_game/src/tes/{player_move,tes_grid,cell_reference_loop,actor_placement,script_commands,script_vm,interface_manager,loading_menu}.rs`, `crates/recomp_game/src/bs/model_loader.rs`, `crates/recomp_startup/src/winmain_menu_media.rs`, the IDB (`~/oblivion-capture/Oblivion.exe.i64`, read only, no debugger), the evidence under `OblivionBotInfos/Recomp/StartupLoading/evidence/gameplay-run00{1,2}-2026-11-07/`, the ESM (`Oblivion.esm` parsed for the quest and script texts), and run-x's `snapshot-after_move` and `snapshot-frame_after_move` (opened as files only).

How to read the numbers. "Own" is the function's size in bytes (hex). "Reach" is bytes of functions reachable by **direct** calls (depth 8, no virtual calls) that the repo never names after an `IDA:` tag and that are not counted: a **lower bound** (virtual calls are invisible; "mentioned only in a comment" is treated as ported by that test, so it undercounts; subtrees overlap heavily: `4E3490`, `667BE0`, `659F30`, `5CA010` all reach the same biped/animation/FaceGen code, so the reaches must not be added). Status words: **ported**, **ported along the reached arms** (other arms stop), **counted** (a `count(world, 0x...)` stand-in), **absent**.

---

## 0. The target state, stated from the original

The task text says "player 3D loaded, camera/first-person active, input to the player, HUD up, loading menu closed". The original's own snapshot of what command 64 ends in (`run-x-aftermove/snapshot-after_move`, taken at `005172C8`, and `frame_after_move`, identical for these fields) shows something narrower:

| Field | Value in `after_move` |
| --- | --- |
| Player cell / position / rotation | cell object `02E0EA40` (`1FBB9`), pos `440DF78B C257AAD3 C2D81F37`, rot `0 0 3FAA238D`; `+0x7F8` = 0 |
| Player 3D | `+0x3C` = `05191D2C` (third-person root); **`+0x5D0` first-person node = `05184CEC`** (set); `+0x588` = 1; `+0x764` = 0 |
| InterfaceManager | status **3**; menu-mode stack top `0x40C` = **1036 = RaceSexMenu**; menus present 1002 Inventory, 1003 Stats, 1004 HUDMain, 1005 HUDInfo, 1022 Magic, 1023 Map, **1036 RaceSex**, 1046 QuickKeys. No 1007 Loading, no 1044 Main. |
| Movie | none (`+0xB33428` = 0) |
| Cell | 468 references, 420 with a 3D node; OSSound `+0x300` map holds 13 sound instances, `+0x304` map 0 |

The x32 frame log (`x32log-newgame-frames.txt`, 766 frames) agrees: status goes 3, 3, 5, 2 and then stays 2 for 262 frames. 3 to 5 to 2 is `00583E60` (cursor shown, `HUDMainMenu_UpdateVisibility(true)`, then 5 becomes 2), i.e. the new menu settling.

Why: `0066EAF0`'s intro block (`+0x7F8`, set by New Game) calls `0057B7E0`, which, when the menu root's user0 is 2.0 (true since Phase 17), calls `sub_5CA010`, a 0x21E1-byte function that reads `Data\Menus\CharGen\race_sex_menu.xml` and builds the **RaceSexMenu**. CharGenQuest's script comment says "bring up race menu first". So the original never reaches "input to the player" unattended: the first stable state is the race menu over the rendered prison.

Two targets, therefore:

* **Target A (milestone):** what the original reaches with no user input: command 64 returns, the player is in `1FBB9` with 3D and first-person node, the Big Four menus and QuickKeys exist, the loading menu is closed, the intro movie has played, the RaceSexMenu is up, status 3 then 5 then 2, and the idle frames run in menu mode with the world rendered behind it. This is `after_move` plus `frame_after_move` and the 260 status-2 frames the x32 log shows.
* **Target B (addendum):** control of the player. Needs the RaceSexMenu's close path (and the rest of chargen's menu flow), after which `PlayerCharacter_UpdateFrame` (`00671620`) gets input and the AI frame runs under `GameMode`. **No evidence run covers it** (nothing was driven past the race menu). Open question 1 asks which target is wanted.

Where "camera enabled" fits: the world scene graph's camera is placed by `0066EAF0` itself (ported: `place_camera`), then refreshed each frame by `0055FA50` (counted) and, once the player has a 3D node and the game isn't in a menu, by `PlayerCharacter_UpdateFrame` → `0066B710` (camera follow, 0xAEB). Under the race menu only the first applies; `0066B710` is Target B.

---

## 1. The path, in the original's call order

Legend for the last column: file::function of the port. All addresses are `Oblivion.exe` 32-bit.

### 1.1 The stop and what is left of command 64

| # | Original (address, own size) | What it does | Status in the port |
| --- | --- | --- | --- |
| 1 | `0066F260` block, tested on `+0x7F8` (`0066EAF0` own 0x87A, the tail 0x10A) | The block below runs only for New Game. | **absent** (`player_move_to_cell` returns the error "0066F260: the intro movie ...") |
| 1a | `0066F27F` `sub_5732D0(g_rendererUIState, 2, 0.3)` (0x488) | Builds fader slot 2 (`Black.dds`): a NiNode, an alpha property, a 4-vertex NiTriShape, a NiSourceTexture from `Data\Textures\Menus\Faders\`, material, attaches it under the UI root, sets its fade time 0.3, flag byte = 1. Reach 0x16FC. | **counted** (`count(.., 0x0057_32D0)` in `tes_grid.rs`; no fader state) |
| 1b | `0066F290` `TES_SetLoadingMenuActive(g_tes, false, false, 0)` (`00440AF0`, 0x14C) | `LoadingMenu_Close` (`00578E30`: removes menu mode 1007, deletes the menu), `HUDSubtitleMenu_HideAndClose` (`005A9010`), `OSSoundGlobals_ResumeFlag10Sounds` (`006A9AA0`..). **The resume draws 10 `rand`s** (`6B6EA1` ← `6A9B24` ← `440C33`, run-ab). | close arm **partly counted** (`00578E30` counted in `tes_grid.rs`, `005A9010` only in `winmain.rs`); ResumeFlag10Sounds **absent** (the port holds no game-sound instances) |
| 1c | `0066F295` `sub_57B7E0` (0x100) | If InterfaceManager, cursor, root exist and root user0 == 2.0 (true): `sub_6A2190(0)` (walk the player's active magic list, `sub_68EA10(0)` for each non-enchant item), `sub_6A2280(0.0)` (update the active effects list, `sub_68E670`), `sub_678D90(target)` (per-manager list reset for the player), `sub_67ACA0(0.0)` (0x1E8, process manager list tick), `unk_B46120/24 = 0`, then `TileMenu_GetByMenuID(RaceSex)` and a `RTDynamicCast` to RaceSexMenu whose vt[0](1) is a **destructor** call on an existing one, then `sub_5CA010()`. Reach 0x27800. | **absent** |
| 1d | `5CA010` (0x21E1) | Builds the RaceSexMenu from `race_sex_menu.xml`: tile template reads, string traits (Race, Face, Hair, Eyes, Complexion, Gender, ...), the player's equipped items (`ContainerChangesData`, `EquippedSlotSelection`), the face preview. **11 `GetRandomPercent` draws** (`4F4EF8` ← `56ACD4` ← `56A593` ← `56A966` ← `5206DC` ← `520745` ×3 ← `521519` ← `64B0DA` ← `5CC1A7` ← `57B8DE`): condition evaluation (`TESCondition_*` is ported) inside `sub_521450` (0x125) / `sub_5206B0` (0xC7) / `sub_64B0B0` (0x55), reached through a process virtual at `[edx+0x384]` called from `5CC1A2`. Reach 0x2CC8B (FaceGen head, biped, tile XML). | **absent** |
| 1e | `0066F2AB` `sub_572EC0(2, 0)` (0x96) | With a=0 and `unk_B3A6D4` = 0 it only clears fader 2's flag byte; with `unk_B3A6D4` != 0 it removes the node and calls `TES_SetLoadingMenuActive(false)` again. | **absent** (mentioned in `gamebryo/renderer.rs` only) |
| 1f | `0066F2B2` `OSSoundGlobals_PauseMusic` (`006A8D00`), `0066F2B9` `PauseGameSounds` (`006A9B40`) | | music **ported** (`os_sound_music.rs`); game-sound pause **absent** (no instances) |
| 1g | `0066F2D1` `MoviePlayer_PlayMovie(sOblivionIntro:General = "OblivionIntro.bik", pollSkip=1, quiet=1, loop=0, preload=0, scaleToWidth=1)` (`00410BA0`, 0x99; `MoviePlayerMovie_Play` pauses/resumes game sounds around it) | Blocking modal playback on the main thread. `Data\Video\OblivionIntro.bik` exists. **Draws 11 `rand`s** in `ResumeGameSounds` (`6B6EA1` ← `6A9CB1` ← `410AF0` ← `410C2E` ← `66F2D6`). | `play_movie`, `play_movie_sequence`, the Bink decode and the host presenter **ported** (`movie_player.rs`, `recomp_host/src/ffmpeg_bink.rs`, `movie_frames.rs`); **the blocking-from-a-script semantics are not** (the startup movies run across host frames); the game-sound resume **absent** |
| 1h | `0066F2DB..0066F2E2` `ResumeMusic`, `ResumeGameSounds`; `+0x7F8 = 0` | | music ported; the rest absent |
| 1i | `0066F2F5` `MoviePlayer_WaitForThread` / `ResumeMusic` only if a movie was already running (`IsRunning`, false here) | | ported arm (`music_pending`) |
| 1j | `0066F316` free `+0x764` via `sub_6B73E0` (0x2D → `57B050`, `57AF10`, `57B0F0`) and `FormHeap_Free`; `+0x764` is 0 after the move | | **absent**; what it holds before is unknown (open question 6) |
| 1k | `0066F332` `g_osGlobals +0x18 = 0` | | **absent** (trivial) |
| 1l | `0066F34D` `sub_498F40(waterSurfaceManager)` for an interior with a water manager: `+0x44 = TESObjectCELL_GetWaterHeight(sub_65E5E0(player, 4096.0))` else 0 | | **absent** (0x2F, reach 0x387) |
| 1m | `0066F358` `TES_InitializeParticleNodeCache` (`00444A10`, 0x259) | Once (`unk_B33A68`): loads about 16 `Effects\*.NIF` (`DustCloud.NIF` and others) synchronously through `sub_439EB0(g_modelLoader, path, 0, 0, 1)` and `TES_CacheParticleNodeVariants`. Reach 0xC7D. | **absent** (the synchronous model load exists as startup infrastructure; this caller does not) |
| 2 | `00512540`'s tail after `0066EAF0` returns (`00512722`): `vt+0x178(0)` = `0065B5C0` Actor_SyncCharControllerWorld (0x190, reach 0x4821), `+0x117 = 1`, `PlayerCharacter_UpdateActiveQuestTargetsForMovedRef` (`00665260`) | The controller sync compares the controller's Havok world with the cell's (`BaseExtraList_GetHavokWorld` for an interior), and re-registers the proxy. | `vt+0x178` **absent for the player** (`move_player_to` ends with the error "00512722: vt+0x178 on the player after 0066EAF0"); `+0x117` present as a field; `00665260` **ported along the no-active-quest arm** (`script_commands::stop_on_active_quest`) |
| 3 | `00512870` returns false (`a3 == target`: the player moved) | The handler reports a failed run, ending the stage's result script. There is nothing after command 64 in that script: CharGenQuest **stage 5**'s results (ESM text) end with `player.moveto CGPlayerStartMarker`. | **ported** (the 64 commands match, `the_port_runs_the_new_game_commands`); the return path past a failing player move is **untested** because the port stops earlier |
| 4 | Unwind: `004FBE00` (result script), `0052ADF0`, `00529AC0` SetStage tail (`0052B080`, `vt+0x40(0x10000000)`), then CharGenQuest's `begin gamemode` continues with `getstage == 5`: every `IF` block in it is false at stage 5. | | ported (`script_commands.rs`, `script_vm.rs`) except that a stage's log entry (`006697A0`) and finishing arm stay stops; stage 5 has neither |
| 5 | The rest of the frame (`0040DBDC` `TES_RunGridScripts` → `004FA580` drain; see 1.3) | | partly ported (`run_grid_scripts_frame`) |

### 1.2 Work that the port passes only through counted stand-ins, inside the cell load that already "runs"

The stop is at `0066F260`, but the state up to there is incomplete in specific, measurable ways. These are the missing pieces **before** the stop, in original order (run-ab chains give the draw sites):

| # | Original | What the port does now | Draws it owes (main thread, run-ab) |
| --- | --- | --- | --- |
| a | `004455E0` TES_EnterInteriorCell → `004D58B0` TESObjectCELL_GetOrCreateNode (0x1D4: the cell's NiNode tree, children by `00B35C00` bits, `004D4A20`) | **counted** (`cell_node` holds only +0x54 = 1) | none |
| b | `004D63A0` AttachToScene (0xA9) → `BaseExtraList_GetHavokWorld`, `004D4D00`, `0088B680(byte_B097B8)` (0x1CB), `004CB670` RefreshReferenceStates (0x50, reach 0x54F), `004CB590` UpdateReferenceSounds (0xD9) → `004D9310` (0x3D6) per sound-bearing reference (type 10/18/24/26 bases) → `006AE4B0` PlaySoundAtPosition (0x26E) → `006AC610` (0x3D8: builds the instance, path `data\sound\...`, voice detection) → `006B6DC0`, `006B6E60` SoundInstance_SetPlaybackEnabled (0xBD) | **counted** (all of them) | 1 at `006A9D8B` (sound region `006A9CD0`, 0x12D) and 11 at `006B6EA1` |
| c | The cell's **Havok world** built in that call (run-ab: about 150 functions in `0088xxxx..0092xxxx`; cover in this session: 119 first-hit Havok functions, 0x528C bytes, on the main thread alone) | **absent**: `cell_has_havok_world` is false, so `tes_update` never meets its "interior's Havok world step" stop | none seen |
| d | `004D5BD0` FinishAttach: `004CB8C0` RunReferenceScripts (0xB5), `004CFAF0` (0x48E, reach 0x11C7), the reference loop | loop **ported** (`cell_reference_loop.rs`, `actor_placement.rs`, including `005FA320` Actor_RebuildHighProcess and its 3 + 8 paths); `004CB8C0`, `004CFAF0`, `004CEAD0`, `004FBF90`'s 0x1000 event over an empty list **counted** | 14 at `00629024` (11) and `005E2F2C` (3): **ported and compared** |
| e | `00438060` ModelLoader_QueueReference (0x29F) × 464 (priority 1) | **counted** (nothing queued) | none directly |
| f | Stage-1 drain (`00445857`, 90 passes) | `TesGridServices::drain` runs the host's timed pass, but with nothing queued it has no work | see g..i |
| g | `0057B950` → `0057E150` BigFour rebuild (0x113) → `0057A440/0057A2D0/0057A180/00579F80` (thunks to Stats `005DCB70` 0x1F9, Inventory `005AB5xx` + `005AADC0` 0x7D3, Magic, Map `005BB6xx` + `005B8FC0` 0x21B, QuickKeys `005C1290` 0x181) each with `LoadingMenu_UpdateDisplay` | **counted** (`BigFourMenuBoundaries`, none of the five menus exists) | 24 at `0058C483` (Stats menu tile trait rolls, inside `005DCB70`) |
| h | The 464 references' 3D: each task's load, then completion `sub_4E3490` (0xA9E, 35 indirect calls) on the clone thread (389 of 419) or the main thread (30, from `00433639` → `0043B06F` → `004421B7`; the actors' through `00659F39`) | **absent** | the 11 actors' `00475D80` (0x2FE; `00474070` 0x495 → `00472030` → `00470BF0/00470C14`) make **108** at `00470C14` on the main thread |
| i | `0069AF30` spell cast (0xB54) from `005EDA20` for the 9 abilities of 5 actors (base list then race list) | `CastBoundaries` records the call; **no ActiveEffect is made** | none attributed in run-ab chains |
| j | `0066ED16`: `006AC210` (0x11B, empties the `+0x304` sound map) and `006AC3C0` (stops flag 0x1000 sounds) | **counted** | none |
| k | `0066ED70` `vt+0x154`: the player has no node, so `IOManager_DrainTasksThroughStage(5)` loads the player's 3D on the main thread: `00667BE0` PlayerCharacter_Load3D (0x44C): `sub_5227A0` equip refresh, `sub_439EB0(g_modelLoader, "Characters\_1stPerson\Skeleton.NIF", 1, 3, 1)`, the **first-person node** (`+0x5D0` "Player1stPerson", `Camera01` at `+0x..`, `0xB3BB0C`), `sub_478730` skin info (0x154-byte object), `sub_473EB0` (0x1BC) anim data, `sub_659F30` (0x5A, third-person `Character` 3D, reach 0x2ADC7), `IdleAnimRoot_FindKFFiles` (ported), `00475D80` ×2, `sub_5E4DD0`, `vt+0x52`, controller hooks | **absent** (the move's code stops at "a drain with no node" = a drain with nothing to load; the player's `+0x3C`/`+0x5D0` stay null) | **24** at `00470C14` (two `00475D80`) |
| l | `0066EEC0`: with a player node `+0x30` takes the pitch matrix | stop ("the player node's rotation") | none |
| m | `0066F09C..0066F1xx`, **because `+0x5D0` is set in the original** (`after_move`): up to 100 iterations of `sub_65AF30(0.16, &zero, 0)` (MobileObject::Move, 0x685, reach 0xA3F) with a break when the Z change is under 0.1; then `+0x588` toggled and `sub_603CA0(this, 0)` twice (0x1ABC), `sub_66B710(this, 0)` (camera follow, 0xAEB) | stop ("the first-person node's placement (0065AF30)") | 1 at `47E086` (the first `603CA0`, chain `6036D0 ← 471E1A ← 471E53 ← 5E103E ← 6043DA ← 70A991 ← 66F20E`), 6 in all; the last 5 are not attributed (see ledger row 7) |
| n | `0066F243` `0055FA50(camera, 0)` (0x258, reach 0x19AA), `007AA4D0` (0x77) | **counted** | none |
| o | `0066F25B` `004424D0(g_tes, frameTime)` (0xFE) | ported for an interior with the counted `006FACA0`, `008984A0`, `004D4970` (0xA1, Havok step, with `889810` 0x211 deciding the substeps), `00707370`, and `00B33A30` | the Sky's **59** at `004EED11` (ported and compared) |

Evidence: the original's position is **not the marker's** in any axis. `CGPlayerStartMarker` is `32AB5`; its runtime position in `after_move`'s cell list equals the ESM's, `440DF784 C257AACC C2CB3224` (567.867, -53.917, -101.598), so no plugin override. The player ends at `440DF78B C257AAD3 C2D81F37`: x and y are each 7 ULPs higher and z is about 6.46 lower (-108.06). The z drop is the `0065AF30` loop (the Havok character controller falling to the floor). The 7-ULP drift in x and y is **a hypothesis**: the controller keeps its position in Havok units (the `0.1428767293691635` scale seen in `0065B5C0`) and writes it back, so a float round trip through that scale would move both. Either way a compare of the player's position needs the `0065AF30` loop or its recorded substitution for all three axes. This is the one place where Havok decides a compared value.

### 1.3 After command 64: the frames until Target A is steady

`0040F26A` loop → `0040D800` `OSGlobals_RunIdleFrame` (0x774), per frame, what matters from `after_move` on (status is 3, so menu mode is true):

1. `0040D809` `IsMenuMode` (ported) and `00889A30` (the Havok-timing menu flag, 0x2E, absent). On the first frame in menu mode after gameplay (`unk_B3341C` 1 to 0): walk the manager's level-0 list (`sub_673A50(0)`), `sub_651DD0` for each high actor (reset animation weights, 0x9E) and for the player's process. **absent**.
2. `0047D170` TimeInfo_Update (ported), `00889810` (substep accumulator, 0x211, absent), `004046A0` input poll (ported), `00433590` IOManager_RunTasksTimed (mentioned, model-load steps ported in `model_loader.rs`).
3. Frame-time globals (`B075E8..F0`) from `sub_43F270`/`sub_4EF1A0` (absent), `007C1F50` (0x96, absent).
4. The menu-background branch: `sub_57AB70` false, `Menu_IsVisibleWithStateMask(RaceSex/Main/Generic/Message)`: a visible RaceSex menu skips the background capture path (partly ported).
5. **Not run in menu mode:** `00411330` (far-plane governor), `004029E0` (game clock), `00440400`. The port skips them too, correctly.
6. `006AE860` OSSoundGlobals_UpdateFrame (ported, 0xDF8; the game-sound arms are absent, reach 0x77C4), `004F5DB0` condition caches (ported), `004402F0` TES_RunGridScripts (ported for what startup reaches; see below).
7. `00674A20` (ported). Then `if (!IsMenuMode || sub_572E30(2))`: `ActorProcessManager_Update` (`00678510`, 0x18C: levels `00673C10` low 0x277, `00673E90` 0x361, `00674200` 0x2F2, `00677EC0` high 0x4C9, reach 0x2A9F7), `00674A20`, `00674950` (0xCD). **Settled from the decompilation: this block does not run under the race menu.** `572E30(2)` is `obj != 0 && flag byte != 0 && alpha == 1.0`, and both arms of `sub_572EC0(2, 0)` at `0066F2AB` kill it (with `unk_B3A6D4` == 0 the flag byte `byte_B12DC8[48]` is cleared; with it nonzero the node is removed and `dword_B12DD0[12]` is 0). Menu mode is true from the intro block on, so neither `ActorProcessManager_Update` nor `004424D0` (item 10) runs. The `OSGlobals_RenderFrame` counter `B33418` (count 1: `TES_SetLoadingMenuActive(false)` and `00674500`; count 10: `572EC0(2,0)`) advances only while `572E30(2)` holds, so those arms belong to the loading-area flow (`unk_B3B72A` → `TES_SetLoadingMenuActive(true,true)`, drain 5, `5732D0(2, flt_B33A48)` in item 11), not to this path. `00572F60` (ported, 0x141) with a fader whose flag is 0 fades its alpha down by `frameTime / fadeTime` and at 0.0 releases the node and clears the slot (`sub_4A2A90` sets the alpha), so fader 2 disappears over about 0.3 s of idle frames: that is the "fader gone" part of Target A.
8. In menu mode: `sub_65E900(target)` (0x60; `sub_4D8370` first-person anim data, `sub_470750(3)`).
9. `0055FA50(camera, menu=1)` (counted), the `TES_UpdateExteriorGrid` branch needs `!menu` (skipped).
10. `if (!menu || 57AB70 || 572E30(2))`: `67ACA0`, `4424D0`: not run for the same reason as 7.
11. `00407AE0` (0x460E: the debug text overlay, not gameplay), `005791A0` AdvanceMenuLoadingStatus (ported along no-HUD/no-console arms), `005791E0`, `00579220`, `unk_B3B72A` (loading area request, not set here).
12. `0040D4D0` OSGlobals_RenderFrame (0x330): `572E30(2)` false resets `B33418`; `sub_579060() != Main && player has 3D && !572DF0(2)` then `sub_40C830` (0x923, the world render), plus `004826F0` (0xC2B, exterior only) and `00411750` (0x420); then `InterfaceManager_RenderIfNoMovie`. The world render of the prison is Target A's visible content. **absent** (the host renders menus and the loading menu only).

Scripts in those frames: post-move, every frame runs about 26 reference scripts (ids `22F85`, `47AC0`, `47AC9`, ..., `CD333`, `4E90E`, `3FAF5`, ...) and the quest scripts (`2480B` CharGenQuest, `297AD`, `36600`, ...), from `004CB8C0` (`TESObjectCELL_RunReferenceScripts`: references with a 3D node or flag `0x800`, non-actors). I counted from the x32 log's 262 status-2 frames: block handlers `5022D0`, `503E10`, `504460`, `503D10`, `504050`, `503F90`, `504150`, `502270`, `5041D0`, `5045A0`, `504560`, `5040D0`, `5044E0`, `504660`, and functions `502BE0` GetStage, `505210` IsPCSleeping, `502890` GetItemCount, plus `5003F0` GetDistance in the first frame. These map to block types 0, 1, 2, 3, 4, 8, 9, 10, 16, 20, 21, 24, 25, 28, 30 in the table at `00B0AF58` (0x28 per row, handler at +0x08); `script_vm.rs::block_predicate` has arms for exactly these (it matches by type, so a grep for the handler addresses finds nothing; it isn't a gap). What **is** a gap:

* **Command 65, `DisableLinkedPathPoints` (`005062D0`, `4395`):** raised by `CGCrumbleWall01SCRIPT` (`3FAF5`, `begin onLoad`), i.e. by a reference's 3D load completing. Needs the OnLoad event raised from the 3D-load completion (`004E3490` → the script event, `004FBF90`), the reference's 3D-loaded gate in `004CB8C0`, `004E51F0(ref, 0)`, and the prison's path grid. `disable_linked_path_points` stops by design ("a loaded cell's path grid").
* The unreached block types (7, 11, 12, 14, 17 to 19, 23) and generic commands stay stops.
* `0040D800`'s script-adjacent calls that run in gameplay frames only: see 1.4.

### 1.4 Target B (not covered by evidence): control of the player

After the RaceSexMenu closes (its own XML, `sub_5CA010`'s companions, not read here): `GameMode` frames with `IsMenuMode` false. `0040D800` then runs `00411330`, `004029E0`, `00440400`, `00673A50` list, `ActorProcessManager_Update`, `target->vt[0x8A]` = `PlayerCharacter_UpdateFrame` (`00671620`, 0x2353, reach 0x95BAA: input via `OSInputGlobals_QueryControlState`, `PlayerCharacter_SetFirstPerson` `0066C580` reach 0x12DB0, `0066B710`, `00603CA0`, activate, grab, fly camera, border check `006714E0`, pause menu), `TES_UpdateExteriorGrid`, `0067ACA0`, `004424D0` with real time, magic projectile root update, then the render. CharGenQuest then proceeds by its stages (stage 6 `Autosave` and Valen Dreth's talk, 9, 14 `disableplayercontrols` ...), so "input to the player" in the ordinary sense also depends on those quest scripts, which are ordinary script VM work. The HUD (`HUDMainMenu`, 1004) exists already in `after_move`; its per-frame `HUDMainMenu_UpdateFrame` (0xEE6) is in the port.

---

## 2. Status of every step (summary)

| Step | Status |
| --- | --- |
| Commands 1 to 64 | ported, compared with `run-v` |
| `00512540` player arm, `0066EAF0` up to `00560110`, camera placement, listener, quest targets, Sky second update | ported along the reached arms (`player_move.rs`), compared only to the extent `after_move` has not yet been used (open-items: "not compared yet") |
| Cell node, attach, Havok world, sounds | counted |
| Reference loop (actors) | ported, compared |
| `00438060`, the 3D loads, animation setup, BigFour, casts | counted or absent |
| Player 3D / first-person node | absent |
| `0065AF30` loop, `00603CA0`, `0066B710`, `0055FA50`, `007AA4D0` | absent or counted |
| Intro block (fader, loading-menu close, `57B7E0`, race menu, movie, `+0x764`, `498F40`, `444A10`) | absent (movie itself ported) |
| `00512540` tail (`vt+0x178`) | absent for the player |
| Idle frame in menu mode | partly ported (clock, input, sound update, scripts, interface advance); AI, TES update gated off; render absent |
| OnLoad events, `DisableLinkedPathPoints` with a cell | absent |
| Target B | absent, no evidence |

---

## 3. Subsystems

Evidence runs: **w** (`records-run-w-rand-cover.txt`: 1,584 first-hit functions from `00512540`, 1,494 on the main thread, 79 on the clone thread 1B8, 11 on 1BC; crashed in the drain), **x** (snapshots `after_move`, `frame_after_move`, draws seed `6AEF4049`), **aa** (480 main-thread draws), **ab** (reference loop, drain entries, 496 draws with 14-word chains, snapshots `fa_ret`, `drain_ret` in `~/oblivion-capture/run-ab-refloop/`), **ac/ad** (the 11 actors in `005FA320`), **x32 frame log** (`gameplay-run001`, 766 frames, `FRAME/BLOCK/CMD/FN/RUN` rows only). Nothing records the main thread past `00434020(5)` except draws and the two snapshots.

From run-w's 1,584 functions (tested against the repo's `IDA:` tags): 721 ported, 689 absent, 149 mentioned only, 25 counted. Main-thread, not-ported, by region (first-hit, so a floor): NiRenderer/NiDX9/accumulator 209 fns / 0x8F69 bytes (host-owned by design), Havok `0088`-`0092` 119 / 0x528C, animation/biped/misc `0046`-`004B` 52 / 0x2766, IO manager queue `0043` 55 / 0x265C, sound `006A`-`006B` 34 / 0x2158, cell/ref/script `004C`-`004F` 43 / 0x1EEA, actor/process/AI/magic `005E`-`0069` 72 / 0x1528, interface `0057`-`005D` 49 / 0xE7F.

### 3.1 Game sounds (instances, cell-attached sounds, pause/resume)
* Key functions: `004CB590` (0xD9), `004D9310` (0x3D6), `006AE4B0` (0x26E), `006AC610` (0x3D8), `006B6DC0`, `006B6E60` (0xBD), `006AB540`, `006ABD..`, `006AC210` (0x11B), `006AC3C0`, `006A9A10` SuspendFlag10Sounds / `006A9AA0` ResumeFlag10Sounds (0x91, confirmed by name at the chain address `006A9B24`), `006A9B40` Pause (0xBA), `006A9C00` ResumeGameSounds (0xCC), `006A9CD0` (0x12D), `006ACCA0`, `006B6C60`. Reach of `4D9310` 0x74E9, `6AE4B0` 0x6D00, `6AC610` 0x60E5 (overlapping, roughly 0xA000 together).
* Dependencies: `OsSoundGlobals` (ported for music: `os_globals.rs`, `os_sound_music.rs`), the host audio (`recomp_host/src/audio.rs`) for actually playing; the INI `bSoundEnabled` (the draws prove it was on in the runs).
* Draws owed in order: 1 + 11 in the attach (`006A9D8B`, `006B6EA1`), 10 at `440C33` (resume flag-0x10 sounds), 11 at `410AF0` (movie resume).
* Evidence: ab (draw chains and counts), `after_move` (13 instances in the `+0x300` map). Needs a new capture: the identity of the 10 flag-0x10 instances (which sound forms, whose references) and the 11 attached ones (form, reference, args to `006AE4B0`).

### 3.2 Model loading and the 3D of the 464 references
* Key functions: `00438060` (0x29F: priority, queue entries `QueuedFile`/`QueuedChildren` family at `0043xxxx`, `00612DE0` 0xC0, `004BCB70`), clone-thread run loop (`00433BC0`, ported as main-thread steps), `004E3490` completion (0xA9E, reach 0x2CDF9: the biggest unknown), `00476D10` (0xE27 biped model attach, reach 0x11FC7), `004E1580` (0x375, reach 0x1E63D), `004DE460` (0x1F6), `00476080` (0x139, reach 0x5026), and per-kind arms (static, light via `004D80C0` already ported, door with `TESSaveLoadGame` flags, container, activator, actor).
* Dependencies: the cell's node (3.9), Havok world (3.5: collision objects attach to it), BS model/NIF stream (ported: `gamebryo/stream_*`, `model_loader.rs` for startup models), controller managers (`gamebryo/controller_sequence.rs`: NiControllerSequence runtime is ported), OnLoad events (scripts).
* Evidence: ab (entries of `4E3490`: 419 runs, 389 clone-thread, 30 main; 464 queues at priority 1), `after_move` (420 of 468 references have a node). **Needs a new capture:** per-call logs inside `4E3490` for one reference of each kind (static, light, door, container, actor, marker with no model) and inside `00667BE0` (run-w died in the drain; run-ab logged entries only). Probe on the main thread only (IO-thread-shared probes crash, `docs/original-debug.md`).

### 3.3 Actor 3D, biped, skeleton, animation setup, process level
* Key functions: `00659F30` (0x5A, reach 0x2ADC7), `00476D10`, `00475D80` (0x2FE) → `00474070` (0x495, reach 0x236D) → `00472030/00472640/00470BF0/00470C14` (the anim-group variant pick, 132 draws in run-ab), `00473EB0` (0x1BC), `00478730`, `005E4DD0` (0x1E9), `005EA1A0`, `006036D0`..`00604xxx` via `00603CA0` (0x1ABC; the port has it only on the LowProcess step-0 arm, `actor_update.rs`; a player on a HighProcess with 3D is a new arm).
* Dependencies: 3.2, FaceGen (`facegen.rs` startup-only), the High process (ported `actor_process.rs`), animation (`controller_sequence.rs`), Havok character controller creation (`character_controller.rs`, ported to its create/sync point).
* Draws: 108 (the 11 actors) + 24 (the player's two setups) at `00470C14`, in run-ab order of completion (needs the model loader's task order to be the original's, including which actors complete on the main thread).
* Evidence: ab (draw sites and counts), ac/ad (the 11 actors until `005FA320`), `after_move`.

### 3.4 Magic: casting, active effects
* Key functions: `0069AF30` (0xB54, reach 0x24895), `005EDA20` (ported shell), `006A2190/006A2280` (0x12A: the active-effects tick called in `57B7E0`), `0068E670`, `0068EA10`.
* Today: `CastBoundaries` records the 9 abilities (5 actors). No effect objects, no actor-value changes from abilities. `after_move` has the resulting values (not compared).
* Dependencies: effect settings (`effect_settings_table.rs`, ported), actor values, `CalculateLuckAdjustedSkill`, `CalculateArmorRatingFromFactors`, `__CIpow` seen in cover; no draws attributed to it.
* New capture: effect lists on the 5 actors' MiddleHigh processes in `after_move`, and `0069AF30` call/returns for the 9 casts (call logging in the style of run-ac).

### 3.5 Havok runtime and the character controller
* Today: Havok setup, layer tables and the bhk stream loaders are ported (`bs/havok.rs`, `havok_layers.rs`, `havok_stream.rs`), and the player's controller is created (`character_controller.rs`, Phase 11). There is **no hkpWorld**: the interior's world and every stepping call are counted or absent.
* Original: the world is built in `004D63A0`/`TESObjectCELL_GetOrCreateNode` (about 150 functions, `0088`-`0092`, the Havok library occupies roughly `0x880000`-`0x980000`), stepped by `004D4970` (0xA1) with substeps from `00889810` (0x211), queried by `bhkCharacterController` (`65AF30`'s `vt+0x80` call, 100 iterations), and kept in sync by `0065B5C0`.
* What the targets actually need: A needs the player's final position (z -108.06 vs the marker's -101.598), the 11 actors' resting positions and the collision registered for references (for `after_move`-state comparison). A step or a raycast result is the only compared value; **no `rand` draw comes from Havok in any run** (the one `006A9D8B` draw is in the sound region).
* Decision needed (open question 2): port enough of hkp for the character controller's down-sweep against the cell's mopp/convex shapes, or record a substitution. Capture: every iteration of `65AF30`'s loop (z before and after, count) and `bhkCharacterController_Move`'s result.

### 3.6 Interface menus (Big Four, QuickKeys, RaceSexMenu, the loading menu's close)
* Key functions: `0057E150` (0x113), `005DCB70` StatsMenu_Create (0x1F9), `005AB5xx` + `005AADC0` (0x7D3), Magic (`005AADC..`, `005B65F0` 0x7E), Map (`005BB6xx`, `005B8FC0` 0x21B, `005BB210` 0x137), QuickKeys (`005C1290` 0x181, `005C1000` 0x5A), `00587240`..; `005CA010` RaceSexMenu (0x21E1, reach 0x2CC8B), `0057B7E0` (0x100).
* Dependencies: the tile/XML machinery (ported: `tile.rs`, `tile_xml.rs`, `menus.rs`, `interface_manager.rs`), the player's inventory and magic lists, FaceGen for the head preview (reach).
* Draws: 24 at `0058C483` (Stats), 11 inside `5CA010` (via process condition evaluation).
* Evidence: ab (the 24 and the five creation entries), `after_move` (menu ids), `x32 frames`. **No run covers `5CA010`'s body**; a call log of it (tile creations in order, and the 11 draws' subjects) is the capture.

### 3.7 Intro movie and the fader
* Key functions: `00410BA0`/`00410AF0` (ported), `005732D0` (0x488), `00572EC0` (0x96), `00572E30`, `00572DF0`, `00572E70`, `00572F60` (ported, 0x141).
* Dependencies: the host movie surface (ported at the startup path), game sounds (3.1) for the 11 draws in `ResumeGameSounds`.
* Evidence: x (the movie is gone after), ab (the 11 draws and their chain).

### 3.8 Camera / first person / player update
* Key functions: `0066EAF0`'s camera block (ported), `0055FA50` (0x258), `0066B710` (0xAEB), `0066C580` SetFirstPerson (0x77, reach 0x12DB0), `00671620` PlayerCharacter_UpdateFrame (0x2353), `00603CA0`.
* Today: `place_camera` ported; the rest absent.
* Evidence: `after_move` (`+0x588` = 1 meaning third-person flag per `PlayerCharacter_GetNiNode`), x32 frames. Capture the `0066B710` call chain on the first menu-mode frame to learn if it runs under the race menu (it should not).

### 3.9 The cell's scene graph and lights
* `004D58B0`, `004D4A20`, `004CA7A0` (0x1C6), `004D80C0` (ported), `004CB670` (0x50, reach 0x54F), `00707370/00707610/00707980` (counted wrappers over ported scene update), `004D4970` (0xA1).
* Evidence: w, ab. Small.

### 3.10 AI and process updates per frame
* `00678510` (0x18C) and the four level updates (reach `677EC0` 0x2A9F7, `673C10` 0x6C2, `673E90` 0x7AC, `674200` 0x73D), `00674A20` (ported), `00674950`, `0067ACA0` (0x1E8), `0060DEA0`, `00683420` (path builder, `g_pathBuilder`, absent). The port's `ai_process.rs` is the tracked-actors pass only; `ActorProcessManager_Update`'s clock `00B3BCF0` never advances (open-items).
* **Needed only after the menu closes, by my reading** (1.3, item 7), so it belongs to Target B, but the Watchman's `+0xFD` and the process-time values compare against it.
* Evidence: run-u (frames 66 to 94, no cell, 1,200 functions), nothing with a cell. Capture needed before porting.

### 3.11 Rendering of the cell
* `0040D4D0` (0x330), `0040C830` (0x923), `004826F0` (0xC2B), `00411750` (0x420), `007B8400`, `007A99A0`, `007D71C0`, accumulator `007A9CF0`/`007AA4D0`/`007AB6F0` (0x267, counted), NiDX9 state (`0077xxxx`, 209 first-hit functions are renderer, mostly host-owned: the port renders through wgpu by design). The cell render needs `BSShaderAccumulator`, light/shadow state, water and sky draws. `OSGlobals_RenderWorldToMenuBackgroundTexture` (`0040D160`, 0x36C) is counted.
* Not needed for parity of values; needed for "rendering of the cell" and for a visible race menu over a world.

### 3.12 Scripts in the loaded cell
* Covered in 1.3. The VM's block and function coverage is sufficient for what the 262 frames show; the OnLoad event, the 3D-loaded gate, `004E51F0` and the path grid are the gaps. Evidence: x32 frames (`RUN`, `CMD`, `FN`, `BLOCK` rows), the ESM texts for `3FAF5` (`CGCrumbleWall01SCRIPT`) and `2480B`.

---

## 4. Suggested order of slices, smallest dependency first

The rand sequence is checked, so a slice that draws must land in order. The **main-thread draw ledger after the loading menu's 192 slides** (run-ab, seed `6AEF61B4`; the chains are the callers `rand` returned to):

| Order | Draws | Site | Port |
| --- | --- | --- | --- |
| 1 | 1 | `006A9D8B` (AttachToScene's sound region) | absent |
| 2 | 11 | `006B6EA1` ← `006AE6DF` ← `004D95E9` ← `004CB64F` ← `004D642D` ← `00445813` (the cell's attached sounds; the chain differs once, `006B6B71`) | absent |
| 3 | 14 | `00629024` ×11, `005E2F2C` ×3 (actors in the reference loop) | **ported, compared** |
| 4 | 24 | `0058C483` (Stats menu, `0057E150`, first drain pass) | absent |
| 5 | 108 | `00470C14` (11 actors' `00475D80`, in the drain) | absent |
| 6 | 24 | `00470C14` (the player's `00667BE0` → `00475D80` ×2, after `00434020(5)`) | absent |
| 7 | 6 | `0047E086`, `0054C452`, `0047FB96`, `0047E086` ×3. The first (`47E086` ← `6036D0` Actor_UpdateHeadTracking ← ... ← `603CA0` ← `66F209`) is the camera block's first `603CA0`. **The other 5 are not attributed:** chains `54C090` (0x534) / `54A120` (0x270: timers from `sub_47E060(0.5, 1.5)`, a 0.3 chance `sub_47FB70`, switched on `sub_54A0B0()` < 13, a weather class) ← `54C6B0` / `55D4C0` ← `70C140` ← `70A12F` recursion. They are `0054xxxx`/`0055xxxx` functions beside the Sky's, so they may be Sky node controllers updating through a scene-graph update rather than the player (candidates: the second `603CA0`, `0066B710`, `004D4970`'s `NiAVObject_UpdateSelected` of the cell node, the camera block's `NiAVObject_Update`). If it is `004D4970`, this row moves from slice 8 to slice 4/5. | absent |
| 8 | 59 | `004EED11` (Sky, `004424D0`) | **ported, compared** |
| 9 | 10 | `006B6EA1` ← `006A9B24` ← `00440C33` (loading menu closes) | absent |
| 10 | 11 | `004F4EF8` (RaceSexMenu `5CA010`: **between 9 and 11**) | absent |
| 11 | 11 | `006B6EA1` ← `006A9CB1` ← `00410AF0` ← `00410C2E` ← `0066F2D6` (movie) | absent |

The 15 draws at `0074FF33` and the 2 at `004E282F` are on the IO and clone threads and are not on this ledger. Rows 4 to 6 depend on the order in which the drain completes loads; 5 and 6 depend on which actors' `004E3490` run on the main thread (30 of 419 in run-ab).

Slices:

1. **Game-sound instances.** Self-contained: SoundInstance, `006AE4B0`/`006AC610`, `004D9310`/`004CB590`, the Suspend/Resume/Pause sets, host audio optional (a host stub that keeps the state and draws exactly). Closes ledger rows 1, 2, 9 (via the loading menu close arm), 11 and 1b/1f/1h. No dependency but `OsSoundGlobals`.
2. **The intro block's shell without the race menu.** `00440AF0`'s close arm, `005732D0`/`00572EC0` state, `+0x764`, `0x18`, `00498F40`, `00444A10`, `play_movie` called from the script, `sub_6B73E0`. Stop at `57B7E0` (or take it as a counted call). Needs 1 for the draws.
3. **Big Four + QuickKeys** (`0057E150`, 5 menus). Closes row 4. Uses the existing tile/XML machinery; the dependency is the player's inventory and magic lists, which exist.
4. **The cell's Havok world and node** (minimal): `004D58B0`, `004D63A0`'s world, `0088B680`, `004D4970`. Decide the Havok depth first (3.5). Without it, collision attachments in 5 have nowhere to go; with a substitution, record it.
5. **Reference 3D: `00438060`, the task queue, `004E3490`** per kind (static, light, door, container, marker, then actor). Closes rows 5 (partly). Order by kind so each lands with its own compare: static (no draws), then lights and doors (OnLoad events and `TESObjectCELL_RunReferenceScripts`'s node gate), then actors (which add `00476D10` and `00475D80`). The per-kind capture (3.2) comes first.
6. **Actor 3D and animation** (`00659F39` path, `00475D80`, `00474070`). Closes row 5.
7. **Player 3D** (`00667BE0`, `00659F30`, first-person skeleton, anim data). Closes row 6. Needs 5 and 6.
8. **Player update arms**: `00603CA0` on a player with 3D, `0065AF30`'s 100-step loop with the Havok controller (4), the second `603CA0`, `0066B710`, `0055FA50`, `007AA4D0`, the `00B3BB..` camera globals. Closes row 7 and the position compare (z -108.06).
9. **Magic** (`0069AF30` and active effects). Independent of the rest; do it before comparing actor values, since the abilities change them. No ledger rows.
10. **RaceSexMenu creation** (`57B7E0`, `5CA010`, `521450`/`5206B0`/`64B0B0` condition evaluation). Closes row 10. Large because of FaceGen; stub the head preview but keep the 11 condition draws in order.
11. **The Target A frames**: menu-mode `0040D800` arms (items 1, 3, 8 in 1.3), the OnLoad event and `DisableLinkedPathPoints` with a loaded cell, status 3 to 5 to 2 with the HUD update, then the world render (host).
12. **Target B** (only if wanted): the race menu close path, `PlayerCharacter_UpdateFrame`, `ActorProcessManager_Update` with all four levels, the real `004424D0`.

Compare checkpoints: after 2, the counts in `COUNTS` of run-ab; after 8, `snapshot-after_move`'s player block (cell, position, `+0x3C`, `+0x5D0`, `+0x588`, `+0x764`); after 10, the menu ids and stack word `0x40C` and status 3; after 11, `frame_after_move`.

---

## 5. Open questions (not settled statically)

1. **Which target?** A (race menu up, no player control) is what the original reaches unattended. B needs a captured run that clicks through the race menu, which no run has. The statement "input to the player" cannot be satisfied by A.
2. **Havok depth.** How much of hkp to port, or whether the character controller can be a recorded substitution. The one value it changes in the compared state is the player's Z (and the actors' resting positions); neither Havok nor the controller draws `rand`. Capture the `65AF30` loop's per-iteration Z and the iteration count.
3. **Per-frame state under the race menu (settled in principle, to be confirmed).** From the decompilation, neither the process manager's update nor `004424D0` runs in menu mode once fader 2's flag is cleared (1.3 item 7), so the Target-A frames are clock, input, sound update, scripts (`TES_RunGridScripts`), interface advance and render. What a capture should confirm: the frames' main-thread draws (expected none besides scripts' `GetRandomPercent`), `unk_B3A6D4`'s value, and that `0066B710` and `ActorProcessManager_Update` have no hits in the 262 status-2 frames.
4. **The 11 draws in the race menu.** Whose process virtual (`[edx+0x384]` at `5CC1A2`, `64B0B0`) and which condition (`GetRandomPercent` with what package) draws 11 times (once per condition or per actor?) is unread; I did not decompile `521450`/`5206B0`.
5. **`+0x7F8` and the movie's blocking.** The original blocks the main thread; the host's movie is a state machine over host frames. How a script-called play returns is a host design item.
6. **`+0x764`** (`0066F316`): freed only if non-null and `g_dataHandler`'s `+0xCD4` flag allows; it is 0 afterwards. What sets it, and whether it holds something at this point, is unread.
7. **Thread order.** Run-w's coverage crashed the IO thread; run-ab's main/clone split (389/30) is an observation under the logger, not proven deterministic. Row 5's order depends on it.
8. **`0057B7E0`'s other effects** (`6A2190`, `6A2280`, `678D90`, `67ACA0(0.0)`, `unk_B46120/24`) on actor values and the process lists before the race menu: unread beyond the headers.
9. **The 13 sound instances** in `after_move`: 11 attached + which 2? (The 10 resumed at `440C33` may be the ones in the `+0x300` map's flagged subset.)
10. **Cell contents compare** ("not compared yet" in open-items): the 468 references, the 420 with nodes, positions and the move's 480 draws against `after_move`.

## 6. Captures to ask for

* C1: main-thread rand with chains and first-hit coverage from `00434020(5)`'s return (`0066ED75`) to `005172C8` and then 10 frames: covers `603CA0`, `65AF30`, `66B710`, `55FA50`, `4424D0`, `5732D0`, `440AF0`, `57B7E0`, `5CA010`, the movie, and the frame body in menu mode.
* C2: the same through the first status-2 frames with coverage per frame (to settle question 3), and `unk_B3A6D4`, `unk_B3341C`.
* C3: inside `004E3490` and `00667BE0`, a per-call log for one reference of each kind, in run-ac's style.
* C4: Havok: `65AF30` iterations; `004D4970` substeps per frame; the controller's final state.
* C5: the sound instances: forms, flags, files for the 13 live and the 10 resumed.
* C6: a driven race-menu close (for Target B), once wanted.
