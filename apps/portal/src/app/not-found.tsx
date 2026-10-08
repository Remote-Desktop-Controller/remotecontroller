import Link from "next/link";
import { PublicShell } from "@/components/portal";
export default function NotFound(){return <PublicShell><div className="container not-found"><p className="eyebrow">404 · PÁGINA NÃO ENCONTRADA</p><h1>Vamos voltar ao início.</h1><p>Este endereço não corresponde a uma página do portal.</p><Link className="button button-dark" href="/">Abrir início</Link></div></PublicShell>;}
