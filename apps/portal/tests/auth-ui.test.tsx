import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {describe,it,expect} from 'vitest';
import {Header} from '../src/components/portal';
import {SessionProvider} from '../src/components/auth-provider';

describe('authentication UI',()=>{
  it('does not claim the visitor is signed out before session verification',()=>{
    const html=renderToStaticMarkup(<SessionProvider><Header/></SessionProvider>);
    expect(html).not.toContain('href="/login"');
    expect(html).not.toContain('href="/register"');
    expect(html).toContain('Verificando sessão');
  });
});
