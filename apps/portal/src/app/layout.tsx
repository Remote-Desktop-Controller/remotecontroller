import type { Metadata } from "next";
import "./globals.css";
import { SessionProvider } from "@/components/auth-provider";
export const metadata:Metadata={title:{default:"Remote Controller — Seu computador. Seu comando.",template:"%s | Remote Controller"},description:"Seu espaço para acompanhar 30 dias de acesso pré-pago ao Remote Controller. O prazo começa no primeiro uso remoto autorizado."};
export default function RootLayout({children}:{children:React.ReactNode}){return <html lang="pt-BR"><body><SessionProvider>{children}</SessionProvider></body></html>;}
