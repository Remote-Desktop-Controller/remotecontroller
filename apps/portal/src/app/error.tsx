"use client";
import { PublicShell, Notice } from "@/components/portal";
export default function ErrorPage({reset}:{reset:()=>void}){return <PublicShell><div className="container not-found"><h1>Vamos tentar de novo.</h1><Notice tone="error">Não foi possível carregar esta página. Nenhuma ação foi confirmada por esta mensagem.</Notice><button className="button button-dark" onClick={reset}>Recarregar página</button></div></PublicShell>;}
