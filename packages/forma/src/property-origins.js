import {expressionReferences} from './expressions.js';

const noOrigins=Object.freeze([]);
// Keep every contributing source for expressions, rather than attributing a
// computed value to an arbitrary last dependency.
// An origin is a source and a label, and both recur for every instance of a
// definition, so each distinct one is built once and carries its dedupe key with
// it. Serializing the key on every call dominated compilation: the key is a
// function of the pair, so a shared pair needs the work only once.
const declarationBySource=new WeakMap();
const declarationByRef=new Map();
const originKeys=new WeakMap();
const has=(origin,key)=>Object.hasOwn(origin,key);
const sourceKeys=new WeakMap();
function sourceKey(source){
 if(source===null||typeof source!=='object')return JSON.stringify(source)??'null';
 const known=sourceKeys.get(source);
 if(known!==undefined)return known;
 const result=JSON.stringify(source)??'null';
 sourceKeys.set(source,result);
 return result;
}
function isCacheable(value){return value!==null&&(typeof value==='object'||typeof value==='function');}
function declaration(source){
 if(!isCacheable(source))return {source,label:source?.label??'Объявление'};
 const known=declarationBySource.get(source);
 if(known!==undefined)return known;
 const result=Object.freeze({source,label:source.label??'Объявление'});
 declarationBySource.set(source,result);
 return result;
}
function referenceOrigin(ref){
 const known=declarationByRef.get(ref);
 if(known!==undefined)return known;
 const result=Object.freeze({label:ref});
 declarationByRef.set(ref,result);
 return result;
}
function originKey(origin){
 const known=originKeys.get(origin);
 if(known!==undefined)return known;
 const result=JSON.stringify([has(origin,'source')?sourceKey(origin.source):null,has(origin,'label')?origin.label:null]);
 originKeys.set(origin,result);
 return result;
}
// A property with one contributing origin repeats for every instance of a
// definition, so its snapshot is shared: keyed by source for literals and by the
// value object for reference-free values.
const originsBySource=new WeakMap();
const originsByValue=new WeakMap();
function singleOrigin(source){
 if(!isCacheable(source))return Object.freeze([declaration(source)]);
 const known=originsBySource.get(source);
 if(known!==undefined)return known;
 const result=Object.freeze([declaration(source)]);
 originsBySource.set(source,result);
 return result;
}
function singleValueOrigin(value,source){
 const known=originsByValue.get(value);
 if(known!==undefined&&known[0]===source)return known[1];
 const result=Object.freeze([declaration(source)]);
 originsByValue.set(value,[source,result]);
 return result;
}
const soleOrigin=(value,source)=>isCacheable(source)?singleValueOrigin(value,source):singleOrigin(source);

export function propertyOrigins(value,source,props={},sources={},traces={},seen){
 if(value===null||typeof value!=='object')return source?singleOrigin(source):noOrigins;
 const references=expressionReferences(value);
 if(references.length===0)return source?soleOrigin(value,source):noOrigins;
 const result=source?[declaration(source)]:[];
 for(const ref of references){
  if(ref.startsWith('props.')){
   const key=ref.slice(6);if(seen?.has(key))continue;
   if(traces[key]){result.push(...traces[key]);continue;}
   seen??=new Set();seen.add(key);
   try{result.push(...propertyOrigins(props[key],sources[key],props,sources,traces,seen));}
   finally{seen.delete(key);}
  }else result.push(referenceOrigin(ref));
 }
 if(result.length<2)return result;
 const unique=[],keys=[];
 for(const origin of result){
  const key=originKey(origin);
  if(keys.includes(key))continue;
  keys.push(key);unique.push(origin);
 }
 return unique;
}

export function displayProperty(value){
 if(value?.expr)return value.expr;
 if(typeof value==='string')return JSON.stringify(value);
 return JSON.stringify(value)??'—';
}
