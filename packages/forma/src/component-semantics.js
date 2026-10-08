// Component semantics executed by the shared Rust language engine.
import {operation} from './rust.js';
import {evaluateExpression} from './expressions.js';
import {propertyOrigins} from './property-origins.js';
export const evaluate=(value,props,state,stack=[],expandElements=true,environment={})=>operation('evaluate',{value,props,state,stack,expandElements,environment});
export const selectedProperties=(groups=[],props,state,environment={})=>operation('selectedProperties',{groups,props,state,environment});
export const selectedPropertySources=(groups=[],props,state,environment={})=>operation('selectedPropertySources',{groups,props,state,environment});
export const evaluateProperties=(node,props,state,environment={})=>operation('evaluateProperties',{node,props,state,environment});
export const validateContract=(definitions={},values,enums={},label='component')=>operation('validateContract',{definitions,values,enums,label});
export const validateTypeName=(type,enums={})=>operation('validateTypeName',{type,enums});
export const matchesType=(value,type,enums={})=>operation('matchesType',{value,type,enums});

// Source annotations and component traversal remain host responsibilities.
export function expandStructure(nodes,props,state,environment={},options={}){
 const result=operation('expandStructure',{nodes,props,state,environment,options},(node,env)=>{
  const annotation={recursive:typeof options.recursive==='function'?options.recursive(node):options.recursive!==false};
  if(options.trackOrigins){
   const sources={...selectedPropertySources(node.matches,props,state,env),...node.propertySources};
   const raw={...Object.fromEntries((node.forward??[]).map(key=>[key,{expr:`props.${key}`}])) ,...selectedProperties(node.matches,props,state,env),...node.props,...Object.fromEntries(Object.entries(node.bindings??{}).map(([key,path])=>[key,{expr:path}]))};
   annotation.propertyOrigins=Object.fromEntries(Object.entries(raw).map(([key,value])=>[key,propertyOrigins(value,sources[key]?{...sources[key],label:`${node.type} · экземпляр`}:node.source,props,options.propertySources)]));
  }
  return annotation;
 });
 function restore(list){for(const node of list){const env=node.environment;if(env?._formaBindings){const locals={...environment.locals};for(const [name,binding]of Object.entries(env._formaBindings)){
  const raw=binding.collection?.expr;let collection;
  if(typeof raw==='string'){
   const parts=raw.split('.'),head=parts.shift();let value=head==='state'?state:head==='props'?props:locals[head];
   for(const part of parts)value=value?.[part];collection=value;
  }
  if(!Array.isArray(collection)){
   const candidates=new Map(),seen=new WeakSet();
   const remember=value=>{if(value===null||typeof value!=='object'||seen.has(value))return;seen.add(value);const key=JSON.stringify(value),previous=candidates.get(key);candidates.set(key,previous===undefined?value:previous===value?value:null);for(const child of Object.values(value))remember(child);};
   const reference=(raw,{optional=false}={})=>{const parts=raw.split('.'),head=parts.shift();let value=head==='state'?state:head==='props'?props:locals[head];for(const part of parts)value=value?.[part];if(value===undefined){if(optional)return null;return {expr:raw};}if(head==='props')value=evaluateExpression(value,reference);remember(value);return value;};
   const computed=evaluateExpression(binding.collection,reference);
   const restoreValue=value=>{if(value===null||typeof value!=='object')return value;const original=candidates.get(JSON.stringify(value));if(original)return original;if(Array.isArray(value))return value.map(restoreValue);return Object.fromEntries(Object.entries(value).map(([key,value])=>[key,restoreValue(value)]));};
   collection=restoreValue(computed);
  }
  locals[name]=Array.isArray(collection)?collection[binding.index]:env.locals[name];
 }env.locals=locals;delete env._formaBindings;}restore(node.children??[]);}}
 restore(result);return result;
}
