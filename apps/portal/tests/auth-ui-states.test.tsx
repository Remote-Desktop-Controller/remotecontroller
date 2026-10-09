import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {describe,it,expect,vi,beforeEach} from 'vitest';
const fixtures=vi.hoisted(()=>({
  auth:{status:'authenticated',user:{id:'fixture',email:'fixture@example.invalid',emailVerified:true,confirmedAt:'2026-10-09T12:00:00Z'},error:'',refresh:vi.fn(),logout:vi.fn()},
  account:{user:{id:'fixture',email:'fixture@example.invalid'},subscription:{state:'NONE',pendingDays:0,activatedAt:null,expiresAt:null},orders:[],serviceAvailable:false,salesEnabled:false}
}));
vi.mock('../src/components/auth-provider',()=>({useAuth:()=>fixtures.auth}));
vi.mock('../src/components/portal',async importOriginal=>({...await importOriginal<typeof import('../src/components/portal')>(),useAccount:()=>({account:fixtures.account,error:'',refresh:vi.fn()}),useConfig:()=>({config:null,error:''})}));
import {Header} from '../src/components/portal';
import {Dashboard} from '../src/components/account-views';
describe('verified session display',()=>{
  beforeEach(()=>{fixtures.auth.status='authenticated';fixtures.auth.user.id='fixture';});
  it('shows panel and logout instead of guest actions for an authenticated session',()=>{
    const html=renderToStaticMarkup(<Header/>);
    expect(html).toContain('Meu painel');expect(html).toContain('Sair da conta');expect(html).not.toContain('href="/login"');expect(html).not.toContain('href="/register"');
  });
  it('shows visitor actions only after a guest session response',()=>{
    fixtures.auth.status='guest';const html=renderToStaticMarkup(<Header/>);
    expect(html).toContain('href="/login"');expect(html).toContain('href="/register"');expect(html).not.toContain('Meu painel');
  });
  it('does not present a session lookup failure as signed out',()=>{
    fixtures.auth.status='error';const html=renderToStaticMarkup(<Header/>);
    expect(html).toContain('Verificar sessão novamente');expect(html).not.toContain('href="/login"');
  });
  it('shows confirmation only for the same authoritative account identity',()=>{
    expect(renderToStaticMarkup(<Dashboard/>)).toContain('E-mail confirmado');
    fixtures.auth.user.id='different-account';expect(renderToStaticMarkup(<Dashboard/>)).not.toContain('E-mail confirmado');
  });
});
