import type { Metadata } from "next";
import "./globals.css";
import { SessionProvider } from "@/components/auth-provider";
export const metadata:Metadata={title:{default:"Remote Controller — Seu computador. Seu comando.",template:"%s | Remote Controller"},description:"Remote Controller: 30 minutos gratuitos por dia e Premium por R$ 9,90 / 30 dias. Acompanhe seu plano e sua conexão."};
export default function RootLayout({children}:{children:React.ReactNode}){return <html lang="pt-BR"><body><SessionProvider>{children}</SessionProvider></body></html>;}
