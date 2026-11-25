declare module "fengari" {
  export type LuaState = object;

  export const lua: {
    readonly LUA_OK: number;
    readonly LUA_MULTRET: number;
    lua_close(state: LuaState): void;
    lua_getglobal(state: LuaState, name: Uint8Array): number;
    lua_pcall(state: LuaState, nargs: number, nresults: number, errfunc: number): number;
    lua_tostring(state: LuaState, index: number): Uint8Array | null;
  };

  export const lauxlib: {
    luaL_loadbuffer(state: LuaState, buffer: Uint8Array, size: null, name: Uint8Array): number;
    luaL_newstate(): LuaState;
  };

  export const lualib: {
    luaL_openlibs(state: LuaState): void;
  };

  export function to_jsstring(value: Uint8Array): string;
  export function to_luastring(value: string): Uint8Array;
}
