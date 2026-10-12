#!/usr/bin/env python3
"""Benchmark pinned local llama.cpp image embeddings; no external connections."""
import argparse
import base64
import io
import json
import os
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path


def main():
    p=argparse.ArgumentParser(__doc__)
    p.add_argument('--runtime',type=Path,required=True)
    p.add_argument('--model',type=Path,required=True)
    p.add_argument('--projector',type=Path,required=True)
    p.add_argument('--manifest',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--limit',type=int)
    args=p.parse_args()
    import numpy as np
    from PIL import Image, ImageOps
    manifest=json.loads(args.manifest.read_text())
    paths=manifest['images'][:args.limit] if args.limit else manifest['images']
    args.output.parent.mkdir(parents=True,exist_ok=True)
    key=secrets.token_urlsafe(24)
    with socket.socket() as reserve:
        reserve.bind(('127.0.0.1',0));port=reserve.getsockname()[1]
    url=f'http://127.0.0.1:{port}'
    env=os.environ.copy()
    env['LD_LIBRARY_PATH']=str(args.runtime.resolve().parent)+':'+env.get('LD_LIBRARY_PATH','')
    log=args.output.with_suffix('.server.log').open('w')
    server=subprocess.Popen([str(args.runtime.resolve()),'--model',str(args.model.resolve()),
        '--mmproj',str(args.projector.resolve()),'--embeddings','--host','127.0.0.1','--port',str(port),
        '--api-key',key,'--ctx-size','8192','--batch-size','8192','--ubatch-size','8192',
        '--threads','2','--threads-batch','2','--parallel','1'],env=env,stdout=log,stderr=log)
    def request(path,body=None):
        req=urllib.request.Request(url+path,data=json.dumps(body).encode() if body is not None else None,
            headers={'Content-Type':'application/json','Authorization':'Bearer '+key})
        with urllib.request.urlopen(req,timeout=120) as response:return json.load(response)
    def embed(content):
        data=request('/v1/embeddings',{'model':'embeddinggemma-2','input':[{'content':content}], 'encoding_format':'float'})
        vector=np.asarray(data['data'][0]['embedding'],dtype=np.float32)
        if vector.shape!=(768,) or not np.isfinite(vector).all() or np.linalg.norm(vector)==0:
            raise ValueError('Invalid native embedding')
        return vector/np.linalg.norm(vector)
    cache=args.output.parent/'prepared-images';cache.mkdir(exist_ok=True)
    try:
        deadline=time.monotonic()+120
        while True:
            if server.poll() is not None:raise RuntimeError('Native server exited; see '+str(args.output.with_suffix('.server.log')))
            try:request('/health');break
            except (urllib.error.URLError,OSError):
                if time.monotonic()>deadline:raise TimeoutError('Native server did not become ready')
                time.sleep(.1)
        vectors,embedded,errors=[],[],[]
        start=time.perf_counter()
        for path in paths:
            try:
                source=Path(path)
                if source.suffix.lower()=='.jxl':
                    import hashlib
                    name=hashlib.sha256(path.encode()).hexdigest()
                    prepared=cache/(name+'.png')
                    if not prepared.exists():
                        partial=cache/(name+f'.{os.getpid()}.png')
                        subprocess.run(['magick',path,'-auto-orient','-thumbnail','768x768',str(partial)],check=True,capture_output=True,timeout=60)
                        partial.replace(prepared)
                    source=prepared
                with Image.open(source) as im:
                    im=ImageOps.exif_transpose(im).convert('RGB');im.thumbnail((768,768))
                    buffer=io.BytesIO();im.save(buffer,format='PNG')
                image_url='data:image/png;base64,'+base64.b64encode(buffer.getvalue()).decode()
                vector=embed([{'type':'image_url','image_url':{'url':image_url}}])
                vectors.append(vector);embedded.append(path)
            except Exception as error:errors.append({'path':path,'error':str(error)})
            if len(embedded)<=3 or len(embedded)%50==0:
                print(json.dumps({'indexed':len(embedded),'errors':len(errors),'seconds':time.perf_counter()-start}),flush=True)
        seconds=time.perf_counter()-start
        if not vectors:raise RuntimeError('No native image embeddings generated')
        matrix=np.asarray(vectors)
        if len(matrix)>1 and np.allclose(matrix[0],matrix[1]):raise RuntimeError('Images produced identical vectors')
        results,latencies=[],[]
        for query in manifest['queries']:
            content=[{'type':'text','text':'task: search result | query: '+query['text']}]
            embed(content)
            began=time.perf_counter();vector=embed(content);scores=matrix@vector
            order=np.argsort(-scores)[:20];latency=time.perf_counter()-began;latencies.append(latency)
            ranked=[embedded[int(i)] for i in order];expected=query.get('expected',[])
            results.append({'text':query['text'],'top20':ranked,'scores':[float(scores[i]) for i in order],
                'latency_seconds':latency,'hit':bool(set(expected)&set(ranked)) if expected else None})
        judged=[r for r in results if r['hit'] is not None]
        report={'images':len(embedded),'index_seconds':seconds,'images_per_second':len(embedded)/seconds,
            'warm_p95_seconds':float(np.percentile(latencies,95)),
            'hit_rate':sum(r['hit'] for r in judged)/len(judged) if judged else None,
            'gate':'pending native/reference parity, complete relevance judgments and navigation measurements',
            'errors':errors,'results':results}
        args.output.write_text(json.dumps(report,indent=2));np.save(args.output.with_suffix('.vectors.npy'),matrix)
        print(json.dumps({k:v for k,v in report.items() if k not in ('results','errors')}),flush=True)
    finally:
        server.terminate()
        try:server.wait(timeout=5)
        except subprocess.TimeoutExpired:server.kill();server.wait()
        log.close()


if __name__=='__main__':main()
