// Host adapters to the Rust frontend. No JavaScript parser or interpreter.
import {operation,expressionRust} from './rust.js';
export const pureFunctions=new Set(['min','max','clamp','abs','round','floor','ceil','len','String','Number','Bool']);
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
