/* tslint:disable */
/* eslint-disable */

export class Game {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    adapter(): string;
    /**
     * Background demo: the player bike drives itself.
     */
    attract(): void;
    backend(): string;
    /**
     * Every bike as x,z pairs, player first.
     */
    bike_points(): Float32Array;
    /**
     * Creates the GPU context on `canvas` (sized by the caller) and
     * loads `slug` in attract mode.
     */
    static boot(canvas: HTMLCanvasElement, slug: string): Promise<Game>;
    cycle_camera(): number;
    /**
     * Advances `dt` seconds of play and draws. Returns the cues raised,
     * as (code, a, b) triples.
     */
    frame(dt: number, throttle: number, brake: number, steer: number, analog_steer: boolean, boost: boolean, attack: boolean): Float32Array;
    hdr_format(): string;
    /**
     * HUD snapshot; layout in `pack_hud`.
     */
    hud(): Float32Array;
    /**
     * The canvas backing store changed size.
     */
    resize(w: number, h: number): void;
    set_paused(paused: boolean): void;
    /**
     * Internal render resolution as a fraction of the canvas (0.35..1).
     */
    set_render_scale(s: number): void;
    /**
     * Loads `slug` (if different) as an attract-mode demo.
     */
    show(slug: string): void;
    /**
     * Standings as `name|time|finished` lines, leader first.
     */
    standings(): string;
    /**
     * Starts a race the player drives, on `slug`.
     */
    start_race(slug: string): void;
    /**
     * Draw calls, triangles and visible props in the last frame.
     */
    stats(): Float32Array;
    /**
     * Current track's slug.
     */
    track(): string;
    /**
     * Centreline as x,z pairs for the minimap.
     */
    track_outline(): Float32Array;
}

/**
 * Which GPU backend this bundle was built for.
 */
export function backend_name(): string;

export function start(): void;

/**
 * The track list as `slug|name|laps|blurb` lines, for the track picker.
 */
export function track_list(): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_game_free: (a: number, b: number) => void;
    readonly backend_name: () => [number, number];
    readonly game_adapter: (a: number) => [number, number];
    readonly game_attract: (a: number) => void;
    readonly game_backend: (a: number) => [number, number];
    readonly game_bike_points: (a: number) => [number, number];
    readonly game_boot: (a: any, b: number, c: number) => any;
    readonly game_cycle_camera: (a: number) => number;
    readonly game_frame: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number];
    readonly game_hdr_format: (a: number) => [number, number];
    readonly game_hud: (a: number) => [number, number];
    readonly game_resize: (a: number, b: number, c: number) => void;
    readonly game_set_paused: (a: number, b: number) => void;
    readonly game_set_render_scale: (a: number, b: number) => void;
    readonly game_show: (a: number, b: number, c: number) => void;
    readonly game_standings: (a: number) => [number, number];
    readonly game_start_race: (a: number, b: number, c: number) => void;
    readonly game_stats: (a: number) => [number, number];
    readonly game_track: (a: number) => [number, number];
    readonly game_track_outline: (a: number) => [number, number];
    readonly start: () => void;
    readonly track_list: () => [number, number];
    readonly wasm_bindgen_fdf1da2e33ce5bc3___convert__closures_____invoke___js_sys_fdd07d1817e7878c___Function_fn_wasm_bindgen_fdf1da2e33ce5bc3___JsValue_____wasm_bindgen_fdf1da2e33ce5bc3___sys__Undefined___js_sys_fdd07d1817e7878c___Function_fn_wasm_bindgen_fdf1da2e33ce5bc3___JsValue_____wasm_bindgen_fdf1da2e33ce5bc3___sys__Undefined_______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_fdf1da2e33ce5bc3___convert__closures_____invoke___wasm_bindgen_fdf1da2e33ce5bc3___JsValue__core_7d5f0a2ba6a62c33___result__Result_____wasm_bindgen_fdf1da2e33ce5bc3___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
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
