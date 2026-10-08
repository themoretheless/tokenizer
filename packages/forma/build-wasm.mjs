import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {resolve,dirname} from 'node:path';
const root=dirname(fileURLToPath(import.meta.url));
function run(command,args){const r=spawnSync(command,args,{cwd:root,stdio:'inherit'});if(r.status!==0)throw Error(`${command} failed`);}
run('cargo',['build','--manifest-path','wasm/Cargo.toml','--locked','--target','wasm32-unknown-unknown','--release']);
run('wasm-bindgen',['wasm/target/wasm32-unknown-unknown/release/forma_language_wasm.wasm','--target','web','--out-dir','wasm-pkg','--out-name','forma_language']);
