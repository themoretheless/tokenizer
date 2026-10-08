const designReference=value=>{const ref=value?.expr??value;return typeof ref==='string'&&ref.startsWith('design.')?ref:null;};
import {evaluateExpression} from './expressions.js';
import {parseRust,operation} from './rust.js';
export const parse=(source,{fragment=false}={})=>parseRust(source,fragment);
export const validateDesign=(component,design)=>operation('validateDesign',{component,design});
export const designStatePatch=(design,stateName=null)=>operation('designStatePatch',{design,stateName});
export function resolve(v,state,readDesignData=ref=>{throw Error(`No design-data resolver supplied for ${ref}`);}){
  const ref=designReference(v);if(ref)return readDesignData(ref);
  if(Array.isArray(v))return v.map(x=>resolve(x,state,readDesignData)).join(' ');
  return evaluateExpression(v,(path)=>{const ref=designReference(path);if(ref)return readDesignData(ref);return path.startsWith('state.')?path.slice(6).split('.').reduce((o,k)=>o?.[k],state):path;});
}
