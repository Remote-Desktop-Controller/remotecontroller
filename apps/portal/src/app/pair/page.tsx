import { redirect } from "next/navigation";
import { getConfig } from "@/lib/config";

export default function PairPage() {
  const config = getConfig();
  const target = config.remoteOrigin ? `${config.remoteOrigin}/pair` : "https://andredozero.webajato.com.br/pair";
  redirect(target);
}
