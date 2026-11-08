# ECS mapping per class family

Added to as each family is ported. The rules are in `migration-plan.md`.

## TESForm head

Original size depends on the class; the head is `+0x00..+0x18`.

| Original | Component or resource | Notes |
| --- | --- | --- |
| `+0x00` vtable | `OriginalVtable(u32)` | Evidence only, never called |
| `+0x04` form type, `+0x08` flags, `+0x0C` form ID | `FormIdentity` | |
| `+0x10` mod reference list | `ModReferences` (mod indexes, list order) | Rows hold raw `TESFile*`; tests compare who shares one |
| `00B0613C` g_formsByID | `FormsById` resource | Size `0x2008D`, head insertion, chain order kept |
| `00B06164` forms by editor ID | `EditorIdMap` resource | 0xFA1 buckets; hash, chain order and `_stricmp` exact. Keys are copies, not the form's pointer |

## TESGlobal (type 0x04, size 0x28)

| Original | Component | Notes |
| --- | --- | --- |
| `+0x18` BSStringT | `EditorId` | Length and capacity (`+0x1C`, `+0x1E`) both come from the text in every recorded row |
| `+0x20` variable type, `+0x24` value | `TESGlobal` | Value kept as raw float bits |
| DataHandler `+0x74` | `DataHandlerLists::globals_74` | `prepend_non_null`: the newest form first |

## Ni objects

Each stream object is an entity. `StreamObjectTable` lists them in stream order, and references index it.

| Original | Component | Notes |
| --- | --- | --- |
| Most-derived class (a `FactoryObject` alternative in C++) | `NiClass` | Link checks test this tag |
| NiObjectNET `+0x08` name, `+0x10` extras, `+0x0C` controller head | `NiObjectNet` | Extras stay sorted by `strcmp`, with later duplicates dropped |
| NiExtraData name | `NiExtraDataName` | `006FF8A0` may generate it during a link |
| NiAlphaProperty `+0x18`, `+0x1A` | `AlphaProperty` | |
| NiAVObject `+0x98` property list | `NiAvProperties` | Head first; null references stay as null nodes |
| NiAVObject `+0x1C` parent | `NiAvParent` | |
| NiAVObject `+0x64` world transform, bounds | `NiAvWorld` | |
| NiNode `+0xAC` child slots | `NiNodeChildren` | An explicit slot array, not Bevy `Children`, because order and null slots are observable |
| NiNode `+0xBC` effect list | `NiNodeEffects` | Head first at >= `04010008` |
| An effect's `+0xC0` affected nodes | `NiEffectAffectedNodes` | |
| The stream load's deferred state | `SceneNodePayload` | Indexes wait here until the link pass |
| NiStringPalette | `StringPalette` | The byte length is the original's capacity |
| NiStringExtraData, NiBinaryExtraData, NiTextKeyExtraData | `StringExtraData`, `BinaryExtraData`, `TextKeyExtraData` | The name is `NiExtraDataName`; text key times are raw bits |
| NiZBufferProperty, NiVertexColorProperty, NiStencilProperty | `ZBufferProperty`, `VertexColorProperty`, `StencilProperty` | Flags keep raw overflow from legacy merges |
| NiMaterialProperty | `MaterialProperty` | IDs from the `MaterialIdCounter` resource (`00B25AF4`) |
| NiWireframeProperty, BSFogProperty, SkyShaderProperty, WaterShaderProperty | Components of those names | Constructor values; render passes keep only counts for now |
| `00714710` factory map | `NiFactoryRegistry` resource | Constructors are identities; `_stricmp` applies only inside one of 59 buckets |

## Textures

| Original | Rust | Notes |
| --- | --- | --- |
| NiSourceTexture, NiSourceCubeMap | `SourceTexture` with `NiObjectNet`, class tag `NiClass::SourceTexture` | One component; `original_vtable` tells the cube apart |
| NiPixelData | `PixelData`, class tag `NiClass::PixelData` | `+0x3C` is `app_pixel_data_3c: Option<Entity>` |
| NiTexturingProperty and its Map, BumpMap, ShaderMap | `TexturingProperty` | Maps are not stream objects: `Option<TexturingMap>` slots inside the component, null slots kept |
| NiDX9SourceTextureData, NiDX9SourceCubeMapData | `TextureRendererData` inside `SourceTexture` | No device objects; everything below is `TextureRendererBoundary` |
| `00B42EBC` / `00B3FAC8` BSTexturePalette | `BsTexturePalette` resource | Explicit bucket lists, head first (1009 and 523). The record map's key is `BsaRecordKey`, a host substitution for `BSFileEntry*` |
## Animation keys and interpolators

Key data and interpolators are separate entities. A plain `update` method takes the resolved key data, and `update_*_interpolator(world, entity, ...)` follows the link. Key types stay raw newtypes (`FloatKeyType(u8)`, `RotationKeyType(u32)`).

| Original | Component | Notes |
| --- | --- | --- |
| NiFloatData, NiBoolData (also NiVisData), NiColorData, NiPosData | `FloatKeyData`, `BoolKeyData`, `ColorKeyData`, `PositionKeyData` | Host key structs, not the original stride ABI; `Vec` holds the keys |
| NiTransformData | `TransformKeyData` | Counts, types and strides at `+08..+1E`; position and scale are `PositionKeyData` and `FloatKeyData` by value; Euler last-index caches live here, so its update takes `&mut` |
| NiFloatInterpolator, NiBoolInterpolator | `FloatInterpolator`, `BoolInterpolator` | `+0x10` data link is `Option<Entity>` |
| NiBoolTimelineInterpolator | `BoolTimelineInterpolator` | `base: BoolInterpolator` plus the two added fields; `NiClass` tells it from a plain bool interpolator |
| NiTransformInterpolator | `TransformInterpolator` | `+0x2C` data link; `QuatTransform` at `+0x0C` |
| NiPathInterpolator | `PathInterpolator` | `+18` path and `+1C` percent links; `+20` partial lengths in a `Vec`, empty for null |
| `007155A0` time mapping's controller fields | `ControllerTimeFields` | A value struct, not a component; a controller holds or converts into it |
| `B27130`, `B3FC88..94` shared time cache | `ParticleControllerTimeCache` resource | The exclusive borrow replaces the C++ mutex |
## Particle data, modifiers and emitters

Each is a stream object with a `NiClass` tag and one component. Update methods are plain functions over `&mut ParticleData`;
the scheduler passes a modifier's linked components and world transforms in, since they live on other entities.

| Original | Component or resource | Notes |
| --- | --- | --- |
| NiPSysData `00759C50` | `ParticleData` | Vectors are the live arrays; an empty optional vector is a null pointer |
| Geometry array claims in a stream allocation group | `ParticleData::allocation_group` (index), `geometry_group_arrays` (offsets) | The group's bytes stay in `StreamAllocationGroups` |
| NiPSysModifier `+0x08`, `+0x0C`, `+0x14` | `ParticleModifierBase`, field `base` of each modifier | The `+0x10` target is consumed, not kept, as in the C++ port |
| Position, GrowFade, AgeDeath, BoundUpdate, Color, Drag, Gravity, Rotation, Spawn | Components of those names, `Particle*Modifier` | AgeDeath `+0x1C` and Color `+0x18` are `Option<Entity>` |
| Drag and gravity `+0x18`, volume emitter `+0x50` raw NiAVObject (13e) | `AvObjectRawRef` | No type check and no reference, as in the original |
| Box, Sphere, Cylinder and Mesh emitters | `Particle*Emitter`, with `state: ParticleEmitterPending` | `state` holds `+0x18..+0x4C` |
| Mesh emitter geometry slots | `ParticleMeshEmitter::linked_geometry_54` | Entities from `link_particle_mesh_emitter`; `mesh_indices` keeps the stream indexes for evidence |
| `00725860` sqrt mantissa lookup | `SqrtMantissaTable` resource | Owned by the singleton tail in `registration.rs`; present exactly while the original's pointer is non-null |
| CRT `rand` | `draw_rand` closures and the `*Services` / `*Runtime` traits | Injected; draw counts and order are kept |

## Mesh geometry and skinning

D3D9 stops here: arrays are host vectors, and no vertex or index buffer exists.

| Original | Component or resource | Notes |
| --- | --- | --- |
| NiGeometryData `00728770` | `MeshGeometryData`, field `base` of each data component | Not a component itself; presence bytes stay raw |
| NiTriShapeData, NiTriStripsData | `MeshTriShapeData`, `MeshTriStripsData` | Group claims in `geometry_group_arrays` / `strip_group_arrays`, as offsets; the bytes stay in `StreamAllocationGroups` |
| NiTriShape, NiTriStrips deferred state | `MeshTriShapePayload` | One payload for both classes (`00719AA0`); the AV prefix shares `av_object::parse_av_object_prefix` with `SceneNodePayload` |
| NiGeometry `+0xAC`, `+0xB0`, `+0xB4`, `+0xB8`, `+0xBC` | `NiGeometry` | Plus `NiObjectNet`, `NiAvProperties`, `NiAvParent` and `NiAvWorld` as on a NiNode. The state and shader slots stay null until their classes are ported |
| NiSkinData, NiSkinPartition | `MeshSkinData`, `MeshSkinPartition` | `MeshSkinPartitionRecord::view` replaces the emitter's former `ParticleMeshSkinPartition` copy |
| NiSkinInstance | `MeshSkinInstance` | `+0x10` root and `+0x14` bones are raw `Option<Entity>`, with no reference, as in the original |
| `0074A390` root and bone worlds | `NiAvWorld` of those entities, read at resolve | A host change from the C++ provider functions; the source and target frames of `make_stream_mesh_emitter_view` likewise |
| A view of owned arrays | `partition_views(world)`, then `view(world, &partitions)` | A view cannot hold its own storage, so the caller keeps the partition views |
| `00B27504` word_B27504 | `GeometryDataIdCounter` resource | Image value 1, 16-bit wrap |
| NiDefaultAVObjectPalette | `DefaultAvObjectPalette` | 37 explicit buckets, head first, `_stricmp` only inside the case-sensitive bucket |
| NiScreenElements, NiScreenElementsData | `ScreenElementsQuad`, class tag `NiClass::ScreenElements` | Host value state, as in C++. `+0xAC` is a `ScenePropertyState`, through `NiGeometry`.|

## Particle system and controllers

Each is a stream object with a `NiClass` tag. Controllers and blends reach other entities, so updates detach the component (`with_detached`) and write it back at every exit.

| Original | Component | Notes |
| --- | --- | --- |
| NiTimeController `+0x08..+0x38` | `ParticleTimeControllerCold`, field `time` of each controller | `fields` is the shared `ControllerTimeFields`; `+0x34` is `next_34`, `+0x30` is `target_30` |
| NiPSysUpdateCtlr, NiPSysEmitterCtlr, NiPSysResetOnLoopCtlr | `ParticleUpdateController`, `ParticleEmitterController`, `ParticleResetOnLoopController` | `controller_time` finds the base by class tag |
| NiPSysModifierActiveCtlr, NiPSysGravityStrengthCtlr | `ParticleModifierActiveController`, `ParticleGravityStrengthController` | `+0x3C` is an `Option<Entity>`, checked by class |
| NiObjectNET `+0x0C` controller list | `NiObjectNet::controller_0c` then `next_34` | One list; the C++ port's separate link node and target state are gone |
| Controller vtable `+0x60` | `MarksGeometryDirty60(bool)` | Evidence for reset only (0069D990, false); absent is a boundary |
| NiBlendBool/Float/TransformInterpolator | `ParticleBlend*Interpolator` | Sources are `Option<Entity>`; `+0x18` is `single_source_18` |
| NiParticleSystem | `ParticleSystem` with `NiObjectNet`, `NiAvProperties`, `NiAvParent`, `NiAvWorld` | `+0xC4` list and `+0xD4` 37-bucket map are explicit |
| Drag and gravity object world transform | `NiAvWorld` of the modifier's raw `+0x18` object | Read at the call, not injected as in C++ |
| BSXFlags, BSParentVelocityModifier | `BsxFlagsExtraData` with `NiExtraDataName`, `BsParentVelocityModifier` | In `bs/nif_objects.rs` |
| `007155A0` shared cache during a schedule | `ParticleControllerTimeCache` through `resource_scope` | Services take `&mut World`, so no closure captures it |
## Scene update, states and scene graphs

Virtual dispatch on the update slots is a match on `NiClass`. Leaves read their local transform from their own payload and store `NiAvWorld`.

| Original | Component or resource | Notes |
| --- | --- | --- |
| NiBillboardNode `+0xDC`, `+0xE0` | `BillboardNode` | With the NiNode components; class tag `NiClass::BillboardNode` |
| NiDirectionalLight | `DirectionalLight` | `+0xBC` stays `NiEffectAffectedNodes`; `+0xCC` is `stream_node_list_cc` |
| `00B259FC` effect UID | `DynamicEffectUidCounter` resource | Image value 1, 32-bit wrap |
| NiPropertyState, NiDynamicEffectState | `ScenePropertyState`, `SceneEffectState` | Entities, because identity is observable; `NiGeometry` `+0xAC` and `+0xB0` hold them. `host_references` stands for the refcount of installs, and a temporary nothing holds is despawned |
| `007317B0` process defaults | `ScenePropertyDefaults` | Read from `NiProcessDefaults::scene_property_defaults` |
| `00B3FA00` property state lock | The `&mut World` borrow | Host substitution |
| A controller's vtable +0x54 | `SceneUpdateRuntime::update_controller` | `SceneControllers` dispatches on `NiClass` and carries the schedule's services; `NoSceneControllers` stops at any head, for tests |
| 0047C930 on a NiParticleSystem | `SceneUpdateRuntime::run_particle_controllers` | `ParticleControllerScheduler`, over the same `ParticleControllerTimeCache` as `ControllerTimeMap::SharedCache` |
| NiParticleSystem +0xAC | `ParticleSystem::property_state_ac` | Set by 00722B30, as on NiGeometry. 00742060's +0xB0 is unported and stays an error, as in the C++ |
| A stream failing after objects exist | `despawn_table`, `despawn_stream_graph` | A host rule for dropping the C++ `shared_ptr` graph. `StreamGraph::palette_textures` (and their +0x3C NiPixelData) stay, because the palette holds them |
| NiCamera | `SceneCamera` | Name, parent, bound, world transform and `+0x98` are the shared components; no factory arm |
| SceneGraph, ShadowSceneNode, BSClearZNode | `SceneGraph`, `ShadowSceneNode`, `ClearZNode` | On a `NiClass::Node` entity, as they share every NiNode slot; the vtable is a field |
| NiCullingProcess, ShadowSceneLight | `CullingProcess`, `ShadowSceneLight` | Owned values inside those components |
| 00406950's globals, `00B39B84`, `00B39E00` | `SceneGraphGlobals`, `CameraStaticAliases` | `Option<Entity>`; `store_trace` holds the store order |
| WinMain `0040E898..0040EAC1` bound | `recomp_startup::winmain_scene::run_scene_window` | Returns each global's `Resource` type in `SceneWindow`; the steps it binds are `src/app/main.cpp`'s in the C++ port |

## Controllers, sequences and blend interpolators

Controller chains, sequence slots and the active list are explicit ordered `Vec`s and `Option<Entity>` links. Nothing walks a query.

| Original | Component | Notes |
| --- | --- | --- |
| NiTimeController `00715990` state | `ParticleTimeControllerCold`, field `time` | Shared with the particle controllers, including `+0x34 next_34` and `+0x30 target_30` |
| Vtable +0x64 time map | `ControllerTimeMap` | `SharedCache` is `007155A0` over `ParticleControllerTimeCache`; `Custom` is a caller's; `Unresolved` is the C++ empty function |
| A NiNode's controllers | `NodeController`, also on NiControllerManager | The chain head is `NiObjectNet::controller_0c`; `next_34` and `target_30` are in `time` |
| NiTransformController, NiVisController `+0x3C` | `TransformController`, `VisController` | `NiClass` tells the interpolator's class |
| NiMultiTargetTransformController | `MultiTargetTransformController` | `+3C` elements are not stream objects, so they stay by value in the component |
| NiControllerSequence | `ControllerSequence` | `InterpArrayItem` and `IDTag` are value structs; `SequenceBlendRef` replaces the raw `+8` blend pointer |
| NiControllerManager | `ControllerManager` with `NodeController` | 37 explicit map buckets, head first. A map key is a copy of the name at insertion, where the original keeps the pointer |
| NiFlipController, NiTextureTransformController | `FlipController`, `TextureTransformController` with `TextureController` | `+44` cached `Map*` is `TextureMapSlot`, a host substitution: it names the slot, so it follows a later replacement of that map |

## Managers and file objects (resources, not entities)

Nothing asks for per-object queries here, and original order lives in explicit lists.

| Original | Rust | Notes |
| --- | --- | --- |
| `00B07BF0` / `00B07D28` / `00B11C40` setting collections | `SettingCollections` resource | The list is head first, as `PrependNonNull` builds it |
| Blend float globals and the `00B2EE68` / `00B2EEE8` tables | `BlendSettingsState` resource | Keyed by original address |
| WinMain's paths (`00B3F178`, `00B3F280`) | `GamePaths` resource | Host paths |
| `00B33A04` FileFinder | `FileFinder` resource | |
| `00B27E4C` / `00B27E50` callback slots | `BsFileCallbacks` resource | Original addresses kept as `u32` values |
| `00B338E0` archive list and its cache and slots | `ArchiveManager` resource | Archives are named by list index |
| Archive's BSFile base, `+0x1A8` count and `+0x200` lock | `ArchiveShared` in an `Arc` | Shared with open `ArchiveFile`s, which outlive any borrow |
| NiFile / BSFile / ArchiveFile / CompressedArchiveFile vtables | `GameFile` trait; `OpenedFile` enum for the open routes | `is_bs_file` stands in for `__RTDynamicCast` to BSFile |

## Startup callbacks and process singletons

The C++ `NiFactoryRegistry` was one object holding all of this. Here each part is a resource, and a callback body is a plain `fn(&mut World)`, so the list is data and runs in its recorded order.

| Original | Resource or component | Notes |
| --- | --- | --- |
| `00747C20` / `00747C40` callback slots, `00747BA0`, `00747BE0` | `StartupCallbackScheduler` | `register_known_crt_callbacks` is the recorded list. Take it with `resource_scope` to run it |
| The scheduler's destructor | `StartupCallbackScope` | `Drop` has no `World`, so this guard borrows one |
| First-run flags of `006C2DB0`, `00748D20`, `00730D80` | `FactoryCallbackGuards` | Cleanup partners clear them |
| `00B3FCE4`.. the refcounted property defaults, `00B3FD28` converter | `NiProcessDefaults` | `Option<Entity>` slots. A release clears the slot and leaves the entity, since owners may still name it and property refcounts are not modelled |
| NiFogProperty, NiShadeProperty, NiRendererSpecificProperty defaults | Components of those names, with `NiObjectNet` | No factory arm and no `NiClass`, like `WireframeProperty`. `FogProperty` is BSFogProperty, a different class |
| `00B3FA90..98` shared fog color | `SharedFogColor` | Read when `00740FA0` runs |
| `00B40134`, `00B3F718`, `00B3F800`, `00B3F928` | `SingletonTail` | The copied words persist after cleanup |
| `00748D20`'s thread ID | `ParticleThread` | `ThreadId`, a host substitution |
| `00B3FCC8..D0`, `00B27164`, `00B27168` | `NiProcessorSpecificState` | In `processor.rs`; never reset |
| `00B42870..80` texgen slots | `NiDx9TexGenSlots` | Identities; nothing reads them |
| `00959F00` tables, with byte `00BA9A65` | `CollisionDispatchState` | Present exactly while the byte is set; targets are identities |
| `0077BEE0` creators, packers, pools, factory maps | `NiD3DShaderCreatorRegistry` | Data only. No Direct3D object exists; M2 owns the renderer |
| `00BA7A00` listener, the WinMain allocation prefix | `StartupMemoryState` in `recomp_startup` | `form_heap_storage` keeps each original null branch reachable |
| `00B35300` palette reference and `00B3FAC8` alias | The `BsTexturePalette` resource | One thing here: its presence is both |

## Screen-element palette

| Original | Rust | Notes |
| --- | --- | --- |
| `00B42CF8` 17 slots (007AB1D0) | `UiScreenElementPalette` resource | Each slot holds a `NiClass::ScreenElements` entity with `NiGeometry`; its `+0xAC` is a `ScenePropertyState`. Property refcounts are not modelled, so `owned` lists what a cleanup releases |
| 00708450 constructor state | `ScreenElementColdGeometry` | Value state, as in the C++ port |

## Renderer

No Direct3D exists. `RendererCreation` owns the result of 00498340 and implements `TextureRendererBoundary`. Calls go through `RendererPlatform`, `Direct3D9` and `Direct3DDevice9`; `device_calls` is their ordered trace.

| Original | Rust | Notes |
| --- | --- | --- |
| NiDX9Renderer (0xB00) and the NiDX9 statics | `RendererState` | Plain values with object offsets in the names. Its object graph is a host substitution |
| `00B06C4C`.. the renderer globals | `RendererGlobals` | |
| INI settings and `.data` constants | `RendererSettings` | `renderer_settings_from` reads them by address |
| NiDX9RenderState caches, `00B2A7C0` defaults | `RenderStateCache`, `RenderStateDefaults` | `render_state_initialization_calls` returns the calls, and the defaults are written back |
| NiDX9AdapterEnumerator and its descriptions | `Dx9SystemDesc`, `Dx9AdapterDesc`, `Dx9DeviceDesc` | An `Arc`, because the callers keep reading an adapter while storing into the renderer. The RT map is a `BTreeMap`, for lookups only, with insertion order in a `Vec` |
| D3DCAPS9 | `D3dCaps9` | Raw dwords in layout order. The answers come from `fixtures/config/device-caps.txt` through `PinnedRendererPlatform` |
| `+0xA98..+0xAC8` notification arrays | `RendererCallbackPairs`, `OriginalPointerArray` | Callbacks are address tokens |
| `00B3A6B0`, `00B12DC8` | `RendererUiState`, `RendererUiGlobals` | |
| D3DX 00760EF5, 007610D3, and 007609C0's level queries | `direct_image_info`, `direct_texture_levels`, on the `device_calls` trace | Host substitution: the DLL's answers from the DDS header. A file needing D3DX's own work (a short chain, a non-power-of-two size) is a host stop |
| A surface write between LockRect and UnlockRect | The bytes passed to `unlock_rect` | Host substitution |
## OS globals, window procedure and FaceGen (Phase 04)

Resources and plain data. Device and Win32 calls go through a seam by original call site; see `docs/porting.md`, "The host boundary".

| Original | Rust | Notes |
| --- | --- | --- |
| `00B33398` g_osGlobals | `OsGlobalsData` resource | Present while non-null. `input_20` and `sound_24` are `Option<Box<..>>`, null on a failed FormHeap request |
| OSInputGlobals `00404150` | `OsInputGlobals` | Devices are `Option<InputDevice>`; DIJOYSTATE, DIMOUSESTATE2 and the rest are plain structs |
| OSSoundGlobals `006ABC80` | `OsSoundGlobals` | COM objects are presence flags; `00B3C214` is a flag on it |
| DirectInput, DirectSound, COM and thread calls | `OsGlobalsPlatform` trait, `ScriptedOsPlatform` for tests | `site` is the call's original address. The script holds the ordered `trace` |
| `00B3C0F0` g_soundSourceGlobal | `SoundSourceGlobal` resource | Names a source, as `SoundSource`; it is non-owning in the original |
| `00B33E90` TimeInfo and the `004060F0` globals | `TimeInfoState` inside `WindowProcedureState` resource | `original_main_window_proc` takes `g_osGlobals` as an argument |
| `004060F0`'s callees | `WindowProcedureServices` trait | DefWindowProc, ShowCursor and the carried `00497BF0`, `00405370` |
| Win32 window class, window and single-instance calls | `recomp_startup::winmain_window`: `WindowPlatform`, and a returned `Vec<WindowCallEntry>` | `recomp_host` builds the Bevy window from the list |
| `00B39B80` FaceGen | `FaceGenSingleton` resource holding `FaceGenState` | Plain matrices and tables. `00761270` is the `create_texture` closure of `FaceGenInputs` |
| `00B3F068` FaceGen last error | `FaceGenLastError` resource | First error sticks; tests make a new one for the C++ reset seam |

## Render targets and effect textures (Phase 05)

No GPU target exists: the 13 targets are records and byte accounting. Device calls go through `Direct3DDevice9` onto a `DeviceTraceEntry` list. Identity is observable (a free-list record names a target, a group shares a buffer), so the `Ni*` objects are entities.

| Original | Rust | Notes |
| --- | --- | --- |
| `00B42F50` BSTextureManager | `TextureManager` in `ShaderManagerGlobals::texture_manager` | The class is in `bs/render_targets.rs`. The lists are `Vec`s in list order. A `RenderedTextureData` is on one list at a time, so the list owns it by value |
| BSRenderedTexture, NiRenderTargetGroup | `RenderedTexture`, `RenderTargetGroup` | `Option<Entity>` links |
| NiRenderedTexture, NiRenderedCubeMap | `RenderedTextureObject` with `NiObjectNet` | `cube` and `original_vtable` tell them apart; a 2D texture's `+0x30` is `buffers[0]`. The renderer data is a value inside |
| Ni2DBuffer, NiDX9TextureBufferData | `RenderBuffer2D` | The data is a value inside, so its `+0x08` back pointer has no field |
| NiDepthStencilBuffer | `DepthStencilBuffer` | The surface is `Option<SurfaceHandle>` |
| `00B2752C`, `00B27530`, the texture and cube counters, renderer `+0x230`, `+0x234` | `RenderTargetGlobals` resource | The maps are insertion-ordered `Vec<Entity>` |
| `00B3FF00`, `00B294EC`, `00B3FAB8` | `&mut` fields of `RenderTargetServices` | Other systems also touch them, so this file does not own them |
| 007B8990's sixteen slots | `EffectTextureGlobals` resource | `Option<Entity>`. `store_trace` holds the store order. A replaced entity stays spawned, as references are not modelled |
| NiScreenPolygon | `ScreenPolygon` | `+0x08` is a `ScenePropertyState` entity |
## Shaders (Phase 05)

Resources and plain data, with no GPU object. `BsShaderContext` holds what the slots read and the pools and maps the original keeps in globals, so a factory takes it and `BsShaderSlots`.

| Original | Rust | Notes |
| --- | --- | --- |
| The five pools `00B42044`, `00B4275C`, `00B427A8`, `00B427AC`, `00B42838` | `ShaderRuntimePools` | Records in slab order; `PassId`, `StageId` and the others are host substitutions for pointers. Free-list order, intrusive counts and header counters are the original's |
| NiD3DPass, NiD3DTextureStage, the two state groups | `ShaderPass`, `ShaderTextureStage`, `ShaderRenderStateGroup`, `TextureStageStateGroup` | A stage's `texture` is the NiSourceTexture entity |
| NiDX9ShaderDeclaration | `ShaderDeclaration` in an `Arc` | Filled before it is shared |
| NiD3DShaderConstantMap | `ShaderConstantMap` in `BsShaderContext::constant_maps`, named by `ConstantMapId` | Slots share maps that later builders still fill; entries are `Arc` |
| NiD3DVertexShader, NiD3DPixelShader | `ShaderProgramObject` in an `Arc` | Keeps `ShaderBytecode`; `Create*Shader` is a `ShaderDeviceCall` in `ShaderProgramServices::device_calls` |
| A local `NiPointer<NiD3DTextureStage>` | `StageRef` | Cannot hold the context, so each scope calls `reset` where the C++ destructor runs, and `Drop` checks it did |
| The slot class chain | `BsShaderObject` as field `base`, reached through `Deref`; `Arc<dyn BsShaderClass>` in `BsShaderSlot` | Identity is `Arc::ptr_eq` |
| `00B42EC0`[28] | `BsShaderSlots` resource | |
| `00B42D7C` | `ImageSpaceShaderList` resource | |
| The class globals | `BsShaderGlobals` in the context | Instance pointers (`00B45FE0`, `00B45F54`, `00B45DCC`) hold the slot index |
| `00B430B8`, `00B430FC`, `00B42F50`, `00B42F31`, `00B294EC`, `00B3FAB8` | `ShaderManagerGlobals` resource | `00B42F50` is the texture family's `TextureManager` |
| DebugShader `+0xBC` | `DebugShader::screen_elements_entity`, or `screen_elements` with no `World` | `get_bs_shader_slot_in_world` threads the `World` through slot 9 only; without one 00707610 and 00707370 are counted |
| `0040E898..0040EAA6`'s globals | `ShaderSystemGlobals` resource | `00B42EBC` is the `BsTexturePalette` resource's presence |
| Op tables of the pass and map builders | `bs/shader_tables.rs` | Generated from the C++ tables |
## MoviePlayer, Bink and pathing (Phase 06)

Resources and plain data. The thread is a real `std::thread` and holds only `MoviePlayer`, a clone of two `Arc`s, never `World`. Device calls leave through `MovieDevice`; `QueuedMovieDevice` puts them on an `mpsc` queue with each tile's pixels at UnlockRect.

| Original | Rust | Notes |
| --- | --- | --- |
| `00B33424..00B33440` | `MoviePlayerGlobals` in `MoviePlayer` | Atomics; the three movie slots and the thread handle are small locks. Identity is `Arc::ptr_eq` |
| MoviePlayerMovie (0x28) | `MoviePlayerMovie` | `+0x20` is an atomic, since 00410425 reads it from either thread. `fields()` locks the rest for one step, never across 00410390 |
| MovieTileSet | `MovieTileSet`, `MovieTile` | The +0 device has no field. A tile's pixels are bytes; the seam receives them at `unlock_rect` |
| `hThread`, CreateThread(CREATE_SUSPENDED) | `MovieThreadHandle`, `HostSemaphore` | A suspended thread blocks on the semaphore. A priority is kept as a value; no scheduler sees it |
| binkw32 | `BinkLibrary`, `ScriptedBink` | Frames and sound come through it. M6 owns the real one |
| Device +0x280 | `MovieDevice`, `ScriptedMovieDevice`, `QueuedMovieDevice` | `site` is the call's original address |
| Win32 and log sinks | `MoviePlatform`, `HostMoviePlatform`, `ScriptedMoviePlatform` | |
| The C++ `os_input_mutex` | `Shared<T>`: `SharedOsInput` (`OsGlobalsData::input_20`), `SharedTimeInfo` (`WindowProcedureState::time`) | A clone names the same state. Lock order is input, then TimeInfo |
| OSSoundGlobals `+0xA4` | `GameSoundPause` | A shared atomic: 00410A70 pauses it on the thread |
| What the thread reads of `00B33398` | `MovieOsGlobals` | Clones of the shared parts, taken from `OsGlobalsData` |
| `00B3A6E0`, the TileMenu table, on another thread | `InterfaceMovieReads` | Atomics, presence only, as 00410390 only tests for null. Phase 07 owns the objects |
| `00B3BE00`, `00B3BF80` | `PathingSingletons` | Plain values with `Option<Box<_>>` for each null branch. Semaphores and the critical section are `PathingHandle`s |
## Interface, fonts and tiles (Phase 07)

Tiles aren't entities: nothing queries by component, every observable order is an explicit list, and evaluating a value needs many others at once. The engine is taken out of the World with `resource_scope` through `with_interface_cx`.

| Original | Rust | Notes |
| --- | --- | --- |
| Tile, Value, Expression, Menu, token lists, `00B3AF10`, `00B3B0A0..A2`, XML caches, `00B13970` | `TileEngine` resource | Plain-data arenas with `TileId`, `ValueId`, `ExprId`, `MenuId` |
| Tile `+0x24` | `Tile::scene_24`, a `TileSceneRecord` | `node` is a `NiClass::Node` entity. `TileSceneGeometry` is the D3 record |
| `00B3A6E0` | `InterfaceManager` resource | Present means non-null. Another thread reads `InterfaceMovieReads` |
| `00B3A6BC` | `FontManager` resource | The block is `Vec<u8>`. `FontInfo +0x0C` holds `PixelData` entities (a substitution for `00704800`'s NiTexturingProperty) |
| `g_rendererUIState +0/+4` | `RendererUiFaderNodes` | The two words `RendererUiState` keeps as unknown |
| `00B42D54` | `MenuModeChanged` | |
| INI, renderer and presence inputs | `InterfaceSettings`, `interface_settings_from` | |
| The C++ `*Services`, 0057F170 | `InterfaceHost`, `InterfaceCx` | `render_interface` is the D3 substitution |

## Sky (Phase 08)

| Original | Rust | Notes |
| --- | --- | --- |
| Sky (0x104), Atmosphere, Stars, Sun, NiPick, Clouds, Precipitation | Plain structs inside `SkyGlobals` (`tes::sky`), with `Option<Entity>` for Ni objects | Only Sky owns the parts |
| `00B365C4`, `00B333B0`, `00B2C73C/40` | `SkyGlobals`: `singleton_b365c4`, `published_b333b0` (a bool), `hdr_*`, `store_trace` | |
| NiGeometry `+0xBC` | `NiGeometryShader` component holding the slot's `Arc` | `Arc::ptr_eq` for 004EC910. Replaces the unused `NiGeometry::shader_bc` |
| ShadowSceneNode `+0x118`, Light `+0x100` | `Option<Box<ShadowSceneLight>>`, `Option<Entity>` | Set by `set_shadow_scene_sun_light` |
| SkyShaderProperty `+0x84`, `+0x88` | Existing component, set by the prelude | 007BDAC0 attaches it with `NiObjectNet` |
| 006F9980 | `BsStreamServices`, `BsStreamLoad` | Built per load, not a resource |

## TES, DataHandler, threads and water (Phase 09)

| Original | Rust | Notes |
| --- | --- | --- |
| `00B333A0` TES, `00B33B00` | `TesGlobals` (`tes`, `save_load_game`) | Grids and water records by value inside. `sky_5c` is a flag, and the Sky is `SkyGlobals` |
| BSTempNodeManager | `BsTempNodeManager` component on a `NiClass::Node` | |
| TLS `+0x184..+0x186` | `GameThreadLocals` | Main thread only |
| Havok globals | `HavokGlobals` | Host substitution for the SDK. bhk factories answer `Unsupported` |
| `00B33A10`, `00B33A1C`, `00B35B8C`/`90` | `TaskGlobals` | Threads hold `Arc<TaskThreadState>`, never `World`. `Drop` joins |
| `00B33A98` DataHandler | `DataHandler` resource, present once published | Skills are entities with `FormIdentity`, `OriginalVtable`, `ModReferences`, `TesSkill` |
| DataHandler-side statics | `DataHandlerGlobals` | |
| `TESFile*` | `TesFileId` into `TesFileStore` | The list is `files_8c8: Vec<TesFileId>`, with explicit order |
| `00B35230` and water statics | `WaterGlobals`; `TesWaterSurfaceRecord`, `TesWaterNodeRecord` by value | |
| `00B39E04` | `TreeManagerGlobals` | |
| WaterShaderProperty attach | `sky_shader::water_shader_prepare` | |

## Plugin selection (Phase 10)

| Original | Rust | Notes |
| --- | --- | --- |
| TESFile `+0x3DC` bits 0, 2, 3 | `TES_FILE_FLAG_IS_MASTER`, `_LOADED`, `_ACTIVE`; `TesFileData::set_loaded` | `0044FAD0` |
| `00447C50` | `lookup_file_by_name(Option<&DataHandler>, Option<&[u8]>)` | `None` is the null original; walks `files_8c8` in list order |
| `00B02C88`, `00A2FF74`, `00B02C90..D8` | `PLUGINS_FILE_NAME`, `FALLBACK_PLUGIN_NAME`, `TEST_FILE_SETTINGS` | Consts in `recomp_startup::plugin_selection` |
| `00404B00`'s CRT stream and char[512] | `TextModeFile`, a local `[u8; 0x200]` | The whole file is read at open |
| `00406687..00406773` | `run_plugin_block`, `run_plugin_selection` | `PluginSelection` is a host record; only the flags stay in the world |

## Player, processes and character controller (Phase 11)

| Original | Rust | Notes |
| --- | --- | --- |
| TESObjectREFR, MobileObject, Actor, Character, PlayerCharacter | Components on one form entity, from `construct_tes_form` | `+0x18` vtable in `TesObjectRefr`, `+0x5C` and `+0x68` in `Actor`, `+0x58` is `MobileObject.process_58` |
| Processes | Entities with `BaseProcess`, `LowProcess`, `MiddleLowProcess`, `MiddleHighProcess`, `HighProcess` and `OriginalVtable` | RTTI casts are component tests; `process_level` reads the vtable. `char_controller_118` is an `Entity` |
| `00B3BD00`, `00B3B928`, `00B3F9A0`, `00B069C3` | `ActorProcessGlobals` | An `ActorList` arena, so `tail_08` and the cursors are real nodes |
| bhk records | Entities: `BhkCharacterController`, `BhkWorldObjectRecord`, `BhkShapeRecord`, `BhkSphereShapeRecord` and two helpers | Each keeps its own `+0x04` count, since shapes and phantoms are shared. The Havok SDK layer is data and traces |
| `00B2EB3C`, `00BA7A54`, `00BA7A64`, cinfo and traces | `CharacterControllerGlobals` | |
| `00B333C4`, `00B3BB24`, `00B3BACC`, `00B3BB38`, `00B14E4D`, `00B3BAFC`, `00B2D80C` | `PlayerGlobals` | |
| DataHandler `+0x8C4`, `+0x8C0` | `active_file_8c4`, `next_form_id_8c0()` over `unknown_8b8[2]` | |
| The 00453450 map | `DataHandlerGlobals.save_load_form_ids` | |

## Data load (Phase 12)

| Original | Rust | Notes |
| --- | --- | --- |
| DataHandler `+0x00..+0xBC`, `+0x8B8` | `DataHandlerLists` | All list heads, with `list_at` and `list_for_type`; `TesObjectList` |
| `+0xCD7` | `DataHandler::loading_cd7` | |
| `00B33A90`, `A94`, `A9C`, `AA0`, `AA4`, `AD8`, `00B06B18` | `DataLoadGlobals` | |
| TESFile `+0x414`, `+0x418` | `decompressed_414`, `decompressed_length_418` | A file is read through `RecordReader`, with the store taken out of `DataHandler` for each step. `flate2` substitutes zlib 1.2.1 |
| Vtable slots | `FormClass`, found by the form's type byte through `form_arms::form_class` | One `CLASSES` table per family |
| BaseFormComponent subobjects | `Tes*` components | Offsets relative to the subobject. `TesObjectLinks` holds `+0x18..+0x20`, and its presence is the TESObject cast |
| A raw form ID slot | `FormRef` | |
| TESClass `+0x38`, TESGlobal `+0x18` | `TesClass`, `EditorId(BsStringT)` | |
| GameSetting `+4` block, collection `+0x04`, `+0x108` | `heap_name`, `byte_04`, `store_file_108` | |
| `0044F3D0` | `LoadSelectedFiles` | A cursor stepping per record; `.run` goes straight through |

## Form families (Phase 12b)

A class's RTTI cast is the presence of its component.

| Original | Rust | Notes |
| --- | --- | --- |
| TESFaction, TESHair, TESEyes, TESSound, TESLandTexture | `TesFaction`, `TesHair`, `TesEyes`, `TesSound`, `TesLandTexture`, with the base components | |
| Script, `00B361AC`, `00B361B0` | `Script`, `ScriptGlobals` | |
| Bound items | `TesActivator`, `TesFurniture`, `TesProduceForm`, `TesApparatus`, `TesArmor`, `TesClothing`, `TesBook`, `TesContainerObject`, `TesDoor`, `TesLight`, `TesSoulGem`, `TesStatic`, `TesGrass`, `TesTree`, `TesWeapon`, `TesAmmo`, `TesSubSpace`, `TesLevItem`, `TesLevCreature` | |
| `00B3FC80`, `84` | `TreeGlobals` | Absent means 0 |
| `00B33508` EffectSettingCollection | `EffectSettingCollection` resource | 0x25 ordered chains, `built_in` in call order. Built at the start of the load, a recorded timing substitution |
| EffectSetting | `EffectSetting`, with `TesModel`, `TesDescription`, `TesFullName`, `TesTexture` | |
| MagicItem part | `EffectItemList`, `MagicFormData`, `TesMagicObject` | INGR and ALCH have both; SGST has the list only |
| TESActorBase | `TesActorBaseData`, `TesContainer`, `TesSpellList`, `TesAiForm`, `TesHealthForm`, `TesAttributes`, `TesAnimation`, `ActorBaseValues` and the name, model and script components | |
| TESNPC, TESCreature, TESRace | `TesNpc`, `TesRaceForm`; `TesCreature`, `TesModelList`, `TesAttackDamageForm`, `CreatureSoundsSlot`; `TesRace`, `TesReactionForm` | |
| TESWeather, TESClimate, TESEffectShader | `TesWeather`, `TesClimate`, `TesEffectShader` | Textures and models are fields |
| TESLoadScreen, TESObjectANIO, TESWaterForm | `TesLoadScreen`, `TesObjectAnio`, `TesWaterForm`, with base components | |
| TESIdleForm, NiFormArray | `TesIdleForm`, `Option<NiFormArray>` | |
| IDLE_ANIM_ROOT, `00B362C0` | `IdleAnimRoots`, indexed by position | Insertion order; the original has 37 buckets |
| TESPackage, TESCondition | `TesPackage`, `PackageLocation`, `PackageTarget`, `PackageSchedule`; `TesCondition` is a `Vec<TesConditionItem>` | |
| TESCombatStyle | `TesCombatStyle` | Byte blocks |
| TESRegion, TESRegionData | `TesRegion`, areas as `Vec<TesRegionPointList>`; region data is an entity with `TesRegionData` | An entity because `00B3541C` outlives its region |
| `00B3541C`, manager `+0x04` | `RegionLoadGlobals` | |
| `00B34D88` | `WarningCounter` | Nothing increments it |
| 00431970 | `DataLoadHost::find_matching_files` | Defaults to empty |

## World, references and dialogue (Phase 12c)

| Original | Rust | Notes |
| --- | --- | --- |
| TESObjectCELL, TESWorldSpace | `TesObjectCell`, `TesWorldSpace` | `toft_50` shares `+0x50` |
| TESRoad, TESObjectLAND, TESPathGrid | `TesRoad`, `RoadPoint`, `TesObjectLand`, `TesPathGrid` | |
| Creature | `Creature` | |
| BaseExtraDataList | `ExtraDataList`, `BaseExtraEntry`, `ExtraData` | Replaces Phase 11's placeholder `data_04` |
| The pointer maps with real buckets | `PointerMap` | |
| `00B35BDC`, `00B35BE0`, `00B35F80`, `00B360A0` | `WorldFormGlobals` | |
| DataHandler `+0xC0`, manager `+0x58` | `NiTLargeArrayCell.cells`, `ActorProcessManager.queued_58` | |
| 0044F179's TOFT arms | `toft_offset_arm` in `data_load` | The `DataLoadHost` seam is gone |
| TESTopic | `TesTopic`, `TesFullName` | `entries_28: Vec<TopicInfoArray>`; infos are `NiFormArray`, quest slots `FormRef` |
| TESTopicInfo | `TesTopicInfo` | `+0x28` and `+0x30` hold raw IDs until InitItem; `+0x34` is `record_offset_34` |
| TESQuest | `TesQuest`, `TesScriptableForm`, `TesTexture`, `TesFullName` | Stage groups, items and targets are plain structs |
| Stage item `+0x0C`, the 00530E1A local, 00B36530 | A `Script` entity owned by the holder | ID 0 and flag 0x4000 on stage scripts; `destroy_script_in_place` despawns it |
| `00B3650C`, `00B36530`, `00B36528`, `00B3652C`, `00B36580`, `00B362FC`, `00B36300` | `DialogueGlobals` | |
| `00B111B8` slots | `DefaultTopicSlots` | |

## Default forms and post-load (Phase 12d)

| Original | Rust | Notes |
| --- | --- | --- |
| vt+0x6C | `InitItemFn`: `fn(&mut World, Entity, &mut DataLoadServices)` | |
| Raw and resolved words | `FormRef`; `BaseExtraEntry.resolved_0c`; `TesConditionItem` `comparison_resolved`, `parameters_resolved`; `ScriptReference.resolved_08` | The raw word stays beside the resolved one |
| Quest `+0x58`, extra `0x12` | `ScriptEventList` | |
| ExtraData variants | `Script`, `StartingPosition`, `ContainerChanges`, `LeveledItem`, `EnableStateChildren` | |
| Inventory | `ContainerChanges`, `InventoryEntry`, `InventoryGlobals` | |
| Statics of `0044AC20` and the tail | `DefaultFormStatics`, `TimeGlobals`, `RaceStatics`, `QuestDelayGlobals` | |
| IDLE_ANIM_ROOT buckets | `IdleAnimRoots.buckets_08` | 37 chains of indexes |
| TESWorldSpace `+0x60`, `+0xD8`; player `+0x704` | `subspaces_60`, `lod_loaded_d8`; `jail_markers_704` | |
| `__RTDynamicCast` by type | `form_hierarchy::form_type_casts_to` | Generated |

## Preload and tasks (Phase 13)

| Original | Rust | Notes |
| --- | --- | --- |
| QueuedModel | `QueuedModel` in `IoTaskMap`, by `IoTaskId` | Plain data, not an entity |
| Model | `Model` in `ModelMap`, by `ModelId` | |
| LockFreeMap entries, TLS slots | Ordered `Vec`s; `LockFreeTlsLock.registered` | |
| `00B33A14`, `18`, `20` | `TaskCounters` | |
| `00B35C80` | `CellRegionLocks` | A `HashMap`, lookups only |
| `00B34428` | `InventoryGlobals.temporary_reference_b34428` | |
| `00B35E50` | `TesGlobals.flame_node_slots_b35e50` | |
| 00433BC0's walk (IO thread) | `ModelDrain`, resumable `step`; `TaskMapIterator` private | Main thread, a host substitution. The caller owns it |
| `QueuedModel`'s gate | `QueuedModel::host_gate`, `ModelHostGate` | State stays 1 |
| `Phase-13e:` counts | `ModelDrainRecord`, with `log_line` for G3 | |
| Model trees as rows | `model_tree_rows` | Walks slots, never a query |
| `00B34404`, `00B34400` | `EditorMarkerState` | Local to one strip, as only one thread runs it here |
| 007D1920 | `classify_geometry_shader` | |
| The IO thread's CRT rand | `MsvcRand` behind `stream.draw_rand` | Per thread, seed 1 |

## AI package pass and world setup (Phase 14)

| Original | Rust | Notes |
| --- | --- | --- |
| `0x21` ExtraRunOncePacks, `0x23` ExtraFollower | `ExtraData::RunOncePacks`, `ExtraData::Followers` | `Vec`, head first; list order is observable |
| `00B36180..` handler caches | `AiProcessGlobals.caches` | Sticky |
| `00B0614C` altered forms | `AlteredForms` | Slot order |
| `0047DF80`, `00578F60` through a seam | `AiArms`, with `AiRealArms`, `AiRecordedArms` | |
| `00B150D4..00B15350` | `PROCEDURE_BLOCK`, `PROCEDURE_ROWS` | Flat, generated from the exe |
| TESSaveLoadGame `+0x00`, `+0x04` maps | `changes_00`, `changes_04`: `ChangeChains` of `ChangeRecord` | Explicit 0x13AF buckets, head first; `changes_map_00` keeps head and count |
| TESWorldSpace `+0x38` LOD map | `PointerMap<TerrainLodEntry>` | Holds `TerrainLodQuad` by value |
| `00B09B20`, `00B09B28`, `00B09B30` | `TerrainLodSettings` | Set by `begin_load_selected_files` |
| `00B36098`, `00B3609C`, `00B46064`, `00B46068`, `00B3608F` | `WorldFormGlobals` | Blocks are held as sizes |
| `00B33A04` vt+4, `00B338E0` from a block | `DataLoadHost::find_file`, `archive_manager` | Host seams |
| WinMain `0040EB31..0040EBB8` | `recomp_startup::winmain_world_setup` | The default point is an output |

## Initial area (Phase 15)

| Original | Rust | Notes |
| --- | --- | --- |
| 004C6170 | `LandData`, boxed in `TesObjectLand.data_24` | |
| 004E7DF0 | `PathGridPoint` in `TesPathGrid.points_24` | Index-linked |
| `B33A69`, `B33A58`, `B2CBC0`, `B3521D`, `B0703C`, `B43384`, `B43074`, `B3522C`, `B4312C..` | `TesGridGlobals` | |
| Counted boundaries | `TesGridBoundaries` | Ordered count map, `percents`, `reference_counts` |
| Answers with no port behind them | `TesGridAnswers` | Seams |
| Slot and TES `+0x58` records | `WaterRecordSlot` | Take and return |
| The block's arms | `InitialAreaArms`, `RealInitialAreaArms` | Replaces the C++ `bind_real_arms` |

## Player placement (Phase 16)

| Original | Rust | Notes |
| --- | --- | --- |
| The block's arms | `PlacePlayerArms`, `RealPlacePlayerArms` | |
| `004CEE90`, `004C3030` results | `PlacementChoice`, `LandHeightTriangle` | |
| `00434020` | `IoDrainResult`, `IoDrainServices` | |
| Player `+0x6E8`, `+0x6EC`, `+0x730` | `region_6e8`, `region_sound_entries_6ec`, `border_region_candidates_730` | |
| Controller `+0x318` | `BhkCharacterController.water_height_318` | |
| `00B33A60`, `00B33A64`, `00B06A2C`, `00B14F58` | `TesGridGlobals` | The two settings are published at `0040EE7A` |

## The host (M6)

| Original | Rust | Notes |
| --- | --- | --- |
| `0040DF80` WinMain's blocks | `Block`, `WinMainProgress`, `run_winmain_frame` | An exclusive chain; new work goes after `Block::Finished` |
| One process's CRT, files and OS | `MainThreadCrt`, `SharedFiles`, `HostOsPlatform` in `WinMainContext` | Non-send, not resources: the movie thread shares only their `Arc`s |
| Device, Bink and DirectShow calls | `WinMainSeams`: `bink`, `movie_device`, `music`, `splash_loader` | Filled by `WinMainPlugin::build` |
| `0057F170`'s arguments | `InterfaceRenderRequests`, `take_render_requests`, `GamePresents` | Drawn by a Bevy system after the frame |
| `00410390`'s gate | `in_scene_flag` | True only around the real draw |
| `0040FA20`'s quads, Present | `MovieFrameCompositor`, `LatestMovieFrame` | A presented picture holds until the next Present |
| binkw32 | `FfmpegBink` | A subprocess and a reader thread |
| OSSoundGlobals' music half | `os_sound_music`, `MusicDevice`, `HostMusicDevice` | |
| `005B5F20`'s media, `0040F092`, `0040F0AA`, `0040F0BE`, `005B5C90`, the idle frame | `winmain_menu_media`, `MainMenuMedia` | The wait is polled |
| DirectInput state | `RawInputFrame`, `feed_raw_input`, `os_input_frame` | |
| Sound output | `HostAudio`, `AudioDeviceReport` | No device degrades and goes on |
| Tile records, text and sprites | `interface_draw`, `recomp_host::screens` | Projected in the host, a stable sort |
| Main, Load, Message menus | `tes::menus`, `tes::interface_frame`, `winmain_screens` | From the real `TileEngine`; the C++'s DOM is retired |
| `0045D5F0`, `00464060` save headers | `tes::save_files` | |
| `005AD440`'s list at `+0x4C` | `loading_menu_slides_at_load_progress`, `slides_4c` | Draws at the tick, since the held `005ADEC0` would run after 12d |
