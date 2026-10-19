// Host adapters to the Rust frontend. No JavaScript parser or interpreter.
import {operation,expressionRust} from './rust.js';
export const pureFunctions=new Set(['min','max','clamp','abs','round','floor','ceil','pow','sqrt','sign','lerp','remap','avg','len','String','Number','Bool','trim','upper','lower','capitalize','padStart','padEnd','truncate','repeat','contains','includes','indexOf','startsWith','endsWith','replace','slice','split','join','reverse','take','skip','sum','range','formatNumber','sortBy','pluck','count','any','all','unique','find','compact','flatten','chunk','zip','first','last','groupBy','keys','values','entries','fromEntries','merge','pick','omit','isEmpty','coalesce','plural','isValid','alpha','mix','lighten','darken','contrast','invert','saturate','desaturate']);
export const tokenize=(source,offset=0)=>operation('tokenize',{source,offset});
export const parseExpression=expressionRust;
export const parseString=expressionRust;
export const equalValues=(a,b)=>operation('equal',{a,b});
export const evaluateExpression=(value,resolveReference)=>operation('evaluateExpression',{value},resolveReference);
export const matchesPattern=(pattern,value,evaluate)=>operation('matchesPattern',{pattern,value},evaluate);
const referenceCache=new WeakMap(),empty=Object.freeze([]);
export function expressionReferences(value){
 if(value===null||typeof value!=='object')return empty;
 let refs=referenceCache.get(value);if(refs===undefined){const result=operation('references',{value});refs=result.length?Object.freeze(result):empty;referenceCache.set(value,refs);}return refs;
}
export const serializeValue=value=>operation('serialize',{value});
