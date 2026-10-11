import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { Pair } from "../src/components/account-views";
import { SessionProvider } from "../src/components/auth-provider";

it("renders Pair component with title, single-use notice, and pairing form", () => {
  const html = renderToStaticMarkup(
    <SessionProvider>
      <Pair />
    </SessionProvider>
  );
  expect(html).toContain("PAREAMENTO DE COMPUTADOR");
  expect(html).toContain("Parear computador");
  expect(html).toContain("Código de pareamento");
  expect(html).toContain("Confirmar pareamento");
  expect(html).toContain("O código expira em dez minutos");
  expect(html).toContain("Parear o computador não inicia seus 30 dias");
});

