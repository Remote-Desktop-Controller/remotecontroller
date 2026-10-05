// Isolated integration fixture; never reads user browser profiles or credentials.
import { chromium, expect } from '../.bootstrap/service-ui/node_modules/@playwright/test/index.mjs';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import assert from 'node:assert/strict';

const executable=resolve(process.argv[2] ?? 'target/release/remote-service.exe');
const fixture=await mkdtemp(join(tmpdir(),'rdc-browser-fixture-'));
const reservation=createServer();
await new Promise(resolve=>reservation.listen(0,'127.0.0.1',resolve));
const port=reservation.address().port;
await new Promise(resolve=>reservation.close(resolve));
const bind=`127.0.0.1:${port}`;
const origin=`http://${bind}`;
const server=spawn(executable,['--bind',bind,'--public-url',origin,'--allow-http-loopback','--database',join(fixture,'service.db')],{windowsHide:true,stdio:['ignore','ignore','pipe']});
let serverFailure=''; server.stderr.on('data',data=>{serverFailure+=String(data);});
let browser;
try {
    let ready=false;
    for(let attempt=0;attempt<100;attempt++) {
        if(server.exitCode!==null) throw Error('Fixture service exited: '+serverFailure);
        try {ready=(await fetch(origin+'/healthz')).ok;} catch {}
        if(ready)break;
        await new Promise(r=>setTimeout(r,100));
    }
    assert(ready,'Service did not become ready');
    browser=await chromium.launch({headless:true});
    const page=await browser.newPage({viewport:{width:390,height:844}});
    page.setDefaultTimeout(10000);
    page.on('request',async request=>{if(request.method()==='POST')console.log('FORM REQUEST',new URL(request.url()).pathname,'Origin='+await request.headerValue('origin'));});
    await page.goto(origin+'/register');
    await page.getByLabel('E-mail').fill('browser-fixture@example.test');
    await page.getByLabel('Senha').fill('Fixture-browser-password-2026!');
    await page.getByRole('button',{name:'Criar conta',exact:true}).click();
    await expect(page.getByRole('heading',{name:'Seus computadores',exact:true})).toBeVisible();
    const pairing=await fetch(origin+'/api/pairings',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({name:'Computador descartável da validação',workspace_id:randomUUID(),tools:[]})});
    assert.equal(pairing.status,201);
    const pending=await pairing.json();
    await page.getByLabel('Código de pareamento').fill(pending.pairing_code);
    await page.getByRole('button',{name:'Confirmar pareamento',exact:true}).click();
    // The real agent receives the credential and creates the device only on poll.
    const delivery=await fetch(origin+'/api/pairings/'+pending.pairing_id+'/poll',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({poll_token:pending.poll_token})});
    assert.equal(delivery.status,200);
    assert.equal((await delivery.json()).status,'approved');
    await page.reload();
    const device=page.locator('article').filter({hasText:'Computador descartável da validação'});
    await expect(device).toBeVisible();
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true,'Mobile overflow');
    await mkdir('docs/product/screenshots',{recursive:true});
    await page.screenshot({path:'docs/product/screenshots/dashboard-mobile-fixture.png',fullPage:true});
    await device.getByRole('button',{name:'Revogar computador',exact:true}).click();
    await expect(device.getByText('Revogado',{exact:true})).toBeVisible();
    await page.getByRole('button',{name:'Sair da conta',exact:true}).click();
    await expect(page.getByRole('heading',{name:'Entrar',exact:true})).toBeVisible();
    await page.getByLabel('E-mail').fill('browser-fixture@example.test');
    await page.getByLabel('Senha').fill('Incorrect-fixture-password-2026!');
    await page.getByRole('button',{name:'Entrar',exact:true}).click();
    await expect(page.getByRole('heading',{name:'Não foi possível continuar',exact:true})).toBeVisible();
    await page.getByRole('link',{name:'Voltar e tentar novamente'}).click();
    await page.getByLabel('E-mail').fill('browser-fixture@example.test');
    await page.getByLabel('Senha').fill('Fixture-browser-password-2026!');
    await page.getByRole('button',{name:'Entrar',exact:true}).click();
    await expect(page.getByRole('heading',{name:'Seus computadores',exact:true})).toBeVisible();
    await expect(page.getByText('Revogado',{exact:true})).toBeVisible();
    await page.setViewportSize({width:1280,height:900});
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true,'Desktop overflow');
    await page.screenshot({path:'docs/product/screenshots/dashboard-desktop-fixture.png',fullPage:true});
    console.log('PASS: real Chromium register, login/error, pairing, device revoke, logout, 390px/1280px layouts');
    console.log('Disposable fixture: '+fixture);
} finally {
    await browser?.close();
    if(server.exitCode===null) {
        server.kill();
        await new Promise(resolve=>server.once('exit',resolve));
    }
    await writeFile(join(fixture,'test-result.txt'),'Fixture preserved; no user account or browser profile was used.\n');
}
