import {ExternalTokenizer} from '@lezer/lr';
import {String as StringToken} from './forma-parser.terms.js';
// Interpolations may themselves contain strings using either quote style.
export const strings=new ExternalTokenizer(input=>{
  function quoted(start){
    const quote=input.peek(start);let i=start+1;
    for(;;){
      const c=input.peek(i);if(c<0)return -1;if(c===92){i+=2;continue;}if(c===quote)return i+1;
      if(c===36&&input.peek(i+1)===123){
        i+=2;let depth=1;
        while(depth){const next=input.peek(i);if(next<0)return -1;if(next===39||next===34){i=quoted(i);if(i<0)return -1;continue;}if(next===123)depth++;if(next===125)depth--;i++;}continue;
      }i++;
    }
  }
  if(input.next!==39&&input.next!==34)return;
  const end=quoted(0);if(end>0){input.advance(end);input.acceptToken(StringToken);}
});
