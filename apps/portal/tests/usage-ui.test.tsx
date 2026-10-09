import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { UsageCard } from "../src/components/usage-card";
it("shows exhausted balance, accessible progress and Brasília renewal",()=>{
  const html=renderToStaticMarkup(<UsageCard usage={{plan:"FREE",remaining_seconds:0,used_seconds:1800,resets_at:1791601200,expires_at:null}}/>);
  expect(html).toContain("Limite gratuito de hoje atingido");
  expect(html).toContain('max="1800"');
  expect(html).toContain('value="1800"');
  expect(html).toContain("Brasília");
  expect(html).toContain("Assinar Premium");
});
it("premium displays expiration and does not suggest consuming Free",()=>{
  const html=renderToStaticMarkup(<UsageCard usage={{plan:"PREMIUM",remaining_seconds:1800,used_seconds:0,resets_at:1791601200,expires_at:1794193200}}/>);
  expect(html).toContain("Premium");
  expect(html).toContain("Sem limite diário de minutos");
  expect(html).toContain("Válido até");
  expect(html).not.toContain("<progress");
});
