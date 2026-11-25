/* tslint:disable */
/* eslint-disable */

/**
 * The booted game: one race, its frame-composition buffers, and the GPU
 * renderer presenting to the loader's canvas. Held across frames so the
 * steady-state loop allocates nothing.
 *
 * wasm-only, and not by choice: wgpu's `SurfaceTarget::Canvas` exists only
 * on wasm targets (wgpu's build script aliases `webgpu`/`webgl` to
 * `not(native)`), so this type cannot compile on host targets. The wasm32
 * checks in the workspace gate compile the real code.
 */
export class BrowserGame {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Advances the race one 60 Hz tick under the loader's input flags and
     * draws the composed frame. Steady state: no allocation after warmup —
     * the batch and road mesh refill through `advance_game_frame`.
     */
    advance_frame(accelerate: boolean, brake: boolean, steer_left: boolean, steer_right: boolean, punch: boolean, kick: boolean): void;
    /**
     * How many bikes are racing, for the HUD's `pos / total` display.
     */
    bike_count(): number;
    /**
     * Boots the demo race on `canvas`: packs the placeholder atlas, selects
     * the quality tier from the loader's capability facts, and brings up the
     * GPU pipeline (warm behind the loading screen per the blueprint).
     *
     * `low_end_device` comes from the loader's device heuristics;
     * `has_webgpu` is compile-time — this build either carries the WebGPU
     * backend or it does not, and the loader already chose the build to
     * match its adapter probe.
     *
     * The canvas is sized before this call; its current `width`/`height`
     * become the swap-chain size.
     *
     * Async factory rather than an async constructor: wasm-bindgen has
     * deprecated async constructors (invalid TS bindings).
     */
    static boot(canvas: HTMLCanvasElement, low_end_device: boolean): Promise<BrowserGame>;
    /**
     * Seconds left on the green-light countdown, for the HUD.
     */
    countdown_secs(): number;
    /**
     * Whether the lights are still red.
     */
    is_counting_down(): boolean;
    /**
     * Whether the player has finished every lap.
     */
    is_finished(): boolean;
    /**
     * Current race lap index, for the loader's HUD.
     */
    lap_index(): number;
    /**
     * Length of one lap in meters, for the HUD's progress display.
     */
    lap_length(): number;
    /**
     * Progress through the player's current lap, in `[0, 1)`, for the HUD's
     * progress bar.
     */
    lap_progress(): number;
    /**
     * Current player distance along the track, in meters, for the HUD.
     */
    player_distance(): number;
    /**
     * The player's combat stamina bar, 0..1, for the HUD strip.
     */
    player_stamina(): number;
    /**
     * The player's race position, 1-based, for the HUD.
     */
    race_position(): number;
    /**
     * Back to the grid: a fresh race with the same track. Inputs and camera
     * state rebuild through the normal per-frame path.
     */
    restart(): void;
    /**
     * The rider's state as a bitmask for the HUD badge: 1 = crashed,
     * 2 = off-road, 4 = braking. A stable u32 rather than a string keeps
     * the per-frame HUD read allocation-free; the loader maps bits to
     * labels and colors. Off-road is derived here the same way the bike
     * state machine draws the line: lateral distance beyond the road's
     * half width.
     */
    rider_state(): number;
    /**
     * Player speed in km/h, for the HUD.
     */
    speed_kmh(): number;
    /**
     * Laps needed to finish, for the HUD's `lap x of N` display.
     */
    total_laps(): number;
}

/**
 * Which backend this build was compiled for, as the loader picked it.
 */
export function backend_name(): string;

/**
 * Installs the panic hook once: panics surface in the browser console
 * instead of vanishing behind a frozen loading screen.
 */
export function start(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_browsergame_free: (a: number, b: number) => void;
    readonly backend_name: (a: number) => void;
    readonly browsergame_advance_frame: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => void;
    readonly browsergame_bike_count: (a: number) => number;
    readonly browsergame_boot: (a: any, b: number) => any;
    readonly browsergame_countdown_secs: (a: number) => number;
    readonly browsergame_is_counting_down: (a: number) => number;
    readonly browsergame_is_finished: (a: number) => number;
    readonly browsergame_lap_index: (a: number) => number;
    readonly browsergame_lap_length: (a: number) => number;
    readonly browsergame_lap_progress: (a: number) => number;
    readonly browsergame_player_distance: (a: number) => number;
    readonly browsergame_player_stamina: (a: number) => number;
    readonly browsergame_race_position: (a: number) => number;
    readonly browsergame_restart: (a: number) => void;
    readonly browsergame_rider_state: (a: number) => number;
    readonly browsergame_speed_kmh: (a: number) => number;
    readonly browsergame_total_laps: (a: number) => number;
    readonly start: () => void;
    readonly __wasm_bindgen_func_elem_1951: (a: number, b: number, c: number, d: any) => void;
    readonly __wasm_bindgen_func_elem_1953: (a: number, b: number, c: any, d: any) => void;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number, b: number, c: number) => void;
    readonly __wbindgen_export4: (a: number) => void;
    readonly __wbindgen_export5: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_export6: (a: number, b: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export7: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
