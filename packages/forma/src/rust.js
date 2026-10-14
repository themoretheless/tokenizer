import init, {parse_json,parse_expression_json,operation_json} from '../wasm-pkg/forma_language.js';
const wasmURL=new URL('../wasm-pkg/forma_language_bg.wasm',import.meta.url);
if(typeof process!=='undefined'&&process.versions?.node){const {readFile}=await import(/* @vite-ignore */ 'node:fs/promises');await init({module_or_path:await readFile(wasmURL)});}else{await init({module_or_path:wasmURL});}
function call(fn,...args){try{return JSON.parse(fn(...args));}catch(error){if(typeof error==='string')throw Error(error);throw error;}}
export const parseRust=(source,fragment=false)=>call(parse_json,source,fragment);
export const expressionRust=source=>call(parse_expression_json,source);
export const operation=(name,input,resolver)=>call(operation_json,name,JSON.stringify(input),resolver);
