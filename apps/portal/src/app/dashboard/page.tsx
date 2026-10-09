import { Dashboard } from "@/components/account-views";
import { requirePageUser } from "@/lib/page-user";
export const dynamic = "force-dynamic";
export default async function Page(){await requirePageUser("/dashboard");return <Dashboard/>;}
