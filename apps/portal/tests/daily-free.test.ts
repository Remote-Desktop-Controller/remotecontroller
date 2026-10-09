import { PGlite } from '@electric-sql/pglite';
import { readFileSync, readdirSync } from 'node:fs';
import { beforeAll, beforeEach, afterAll, it, expect } from 'vitest';
const user = '33333333-3333-4333-8333-333333333333', other = '44444444-4444-4444-8444-444444444444', initial = '2026-10-09T15:00:00Z';
let db: PGlite;
type Usage = {
    allowed: boolean;
    state: string;
    plan: string;
    expires_at: number | null;
    remaining_seconds: number;
    resets_at: number;
    error_code: string | null;
    session_active_until: number | null;
};
const epoch = (s: string) => Date.parse(s) / 1000;
async function time(t: string) {
    await db.query('update test_clock set instant=$1::timestamptz', [t]);
}
async function usage(action = 'reserve', id = crypto.randomUUID(), subject = user, max = 120, available = true, orders: string[] = []) {
    return (await db.query<{
        result: Usage;
    }>('select portal_usage($1::uuid,$2::uuid,$3::text,$4::integer,$5::boolean,$6::uuid[]) result', [subject, id, action, max, available, orders])).rows[0].result;
}
async function status(subject = user) {
    return (await db.query<{
        result: Usage;
    }>('select portal_usage_status($1::uuid) result', [subject])).rows[0].result;
}
async function paid() {
    const id = crypto.randomUUID();
    await db.query("insert into portal_orders(id,user_id,amount_cents,status,creation_state) values($1,$2,990,'PAID_AWAITING_SERVICE','READY')", [id, user]);
    await db.query('insert into portal_entitlements(user_id,order_id) values($1,$2)', [user, id]);
    return id;
}
beforeAll(async () => {
    db = new PGlite();
    await db.exec(`create role anon;create role authenticated;create role service_role bypassrls;create schema auth;create table auth.users(id uuid primary key);insert into auth.users values('${user}'),('${other}');create function auth.uid() returns uuid language sql stable as $$select null::uuid$$;create table test_clock(instant timestamptz);insert into test_clock values('${initial}');create function public.test_now() returns timestamptz language sql stable as $$select instant from test_clock$$;revoke all on test_clock from public,anon,authenticated,service_role;revoke all on function public.test_now() from public,anon,authenticated,service_role;`);
    // Test-only clock substitution: the production RPC accepts no time or elapsed duration.
    const dir = new URL('../supabase/migrations/', import.meta.url);
    for (const file of readdirSync(dir).filter(f => f.endsWith('.sql')).sort())
        await db.exec(readFileSync(new URL(file, dir), 'utf8').replace(/\bnow\(\)|\bclock_timestamp\(\)/g, 'public.test_now()'));
}, 30000);
beforeEach(async () => {
    await db.exec('truncate portal_usage_operations,portal_usage_sessions,portal_operations,portal_events,portal_entitlements,portal_orders,portal_test_grants cascade');
    await time(initial);
});
afterAll(async () => {
    await db.close();
});
it('installs the Free usage RPC', async () => {
    expect((await db.query<{
        fn: string | null;
    }>("select to_regprocedure('public.portal_usage(uuid,uuid,text,integer,boolean,uuid[])') fn")).rows[0].fn).not.toBeNull();
});
it('inspect and status do not write quota or financial records', async () => {
    expect(await usage('inspect')).toMatchObject({ allowed: true, plan: 'FREE', remaining_seconds: 1800, expires_at: epoch(initial) + 120 });
    expect(await status()).toMatchObject({ remaining_seconds: 1800, session_active_until: null });
    for (const table of ['portal_usage_operations', 'portal_orders', 'portal_operations', 'portal_entitlements', 'portal_events'])
        expect((await db.query(`select * from ${table}`)).rows).toHaveLength(0);
});
it('reserves at dispatch and charges SQL elapsed time at complete', async () => {
    const id = crypto.randomUUID();
    expect(await usage('reserve', id)).toMatchObject({ allowed: true, remaining_seconds: 1680, expires_at: epoch(initial) + 120 });
    await time('2026-10-09T15:00:25Z');
    expect(await usage('complete', id)).toMatchObject({ allowed: false, state: 'COMPLETED', remaining_seconds: 1775 });
    expect(await usage('reserve', id)).toMatchObject({ allowed: false, state: 'COMPLETED', remaining_seconds: 1775 });
});
it('unions concurrent intervals rather than summing overlapping work', async () => {
    const a = crypto.randomUUID(), b = crypto.randomUUID();
    await usage('reserve', a);
    await time('2026-10-09T15:00:30Z');
    expect((await usage('reserve', b)).remaining_seconds).toBe(1650);
    await time('2026-10-09T15:01:00Z');
    await usage('complete', a);
    await time('2026-10-09T15:01:30Z');
    expect((await usage('complete', b)).remaining_seconds).toBe(1710);
});
it('serializes competing reservations without charging their overlap twice', async () => {
    const all = await Promise.all(Array.from({ length: 8 }, () => usage()));
    expect(all.every(r => r.allowed && r.remaining_seconds === 1680)).toBe(true);
});
it('keeps abandoned reservations conservative and rejects expired replay', async () => {
    const id = crypto.randomUUID(), first = await usage('reserve', id);
    await time('2026-10-09T15:03:00Z');
    expect(await usage('reserve', id)).toMatchObject({ allowed: false, state: 'EXPIRED', remaining_seconds: 1680, expires_at: first.expires_at });
    expect((await usage('complete', id)).remaining_seconds).toBe(1680);
});
it('replay cannot extend a deadline or change account', async () => {
    const id = crypto.randomUUID(), first = await usage('reserve', id, user, 60);
    await time('2026-10-09T15:00:10Z');
    expect((await usage('reserve', id)).expires_at).toBe(first.expires_at);
    expect(await usage('reserve', id, other)).toMatchObject({ allowed: false, error_code: 'OPERATION_ID_CONFLICT' });
    expect((await status(other)).remaining_seconds).toBe(1800);
});
it('exhausts 1800 seconds including reservations while permitting already reserved overlap', async () => {
    for (let i = 0; i < 15; i++) {
        await time(new Date(Date.parse(initial) + i * 120000).toISOString());
        expect((await usage()).allowed).toBe(true);
    }
    expect((await usage()).allowed).toBe(true);
    await time('2026-10-09T15:30:00Z');
    expect(await usage()).toMatchObject({ allowed: false, remaining_seconds: 0, error_code: 'FREE_DAILY_LIMIT_REACHED' });
    expect((await usage('inspect')).allowed).toBe(false);
});
it('clips the final reservation to the remaining quota', async () => {
    for (let i = 0; i < 14; i++) {
        await time(new Date(Date.parse(initial) + i * 120000).toISOString());
        await usage();
    }
    await time('2026-10-09T15:28:00Z');
    await usage('reserve', crypto.randomUUID(), user, 110);
    await time('2026-10-09T15:29:50Z');
    expect(await usage()).toMatchObject({ allowed: true, remaining_seconds: 0, expires_at: epoch('2026-10-09T15:30:00Z') });
});
it('caps at Sao Paulo midnight and resets without resurrecting old IDs', async () => {
    await time('2026-10-10T02:59:50Z');
    const id = crypto.randomUUID();
    expect(await usage('reserve', id)).toMatchObject({ remaining_seconds: 1790, expires_at: epoch('2026-10-10T03:00:00Z'), resets_at: epoch('2026-10-10T03:00:00Z') });
    await time('2026-10-10T03:00:00Z');
    expect((await status()).remaining_seconds).toBe(1800);
    expect((await usage('reserve', id)).allowed).toBe(false);
    expect((await usage()).remaining_seconds).toBe(1680);
});
it('unavailability cannot spend quota or start a session', async () => {
    expect(await usage('reserve', crypto.randomUUID(), user, 120, false)).toMatchObject({ allowed: false, error_code: 'SERVICE_UNAVAILABLE', session_active_until: null });
    expect((await status()).remaining_seconds).toBe(1800);
});
it('status stays Free while inspect offers verified pending Premium without activating', async () => {
    const order = await paid();
    expect((await status()).plan).toBe('FREE');
    expect((await usage('inspect', crypto.randomUUID(), user, 120, true, [order])).plan).toBe('PREMIUM');
    expect((await db.query<{
        activated_at: string | null;
    }>('select activated_at from portal_entitlements')).rows[0].activated_at).toBeNull();
    expect(await usage('reserve', crypto.randomUUID(), user, 120, true, [order])).toMatchObject({ plan: 'PREMIUM', allowed: true, remaining_seconds: 1800 });
    expect((await db.query<{
        duration: number;
    }>('select extract(epoch from expires_at-activated_at)::int duration from portal_entitlements')).rows[0].duration).toBe(2592000);
});
it('unverified paid records never activate or bypass Free quota', async () => {
    await paid();
    expect((await usage()).plan).toBe('FREE');
    expect((await db.query<{
        activated_at: string | null;
    }>('select activated_at from portal_entitlements')).rows[0].activated_at).toBeNull();
});
it('Premium expiry returns to Free and never resurrects replay', async () => {
    const order = await paid(), id = crypto.randomUUID();
    await usage('reserve', id, user, 120, true, [order]);
    await time('2026-11-08T15:00:00Z');
    expect((await usage('reserve', crypto.randomUUID(), user, 120, true, [order])).plan).toBe('FREE');
    expect((await usage('reserve', id, user, 120, true, [order])).allowed).toBe(false);
});
it('Admin grants do not spend quota or activate queued paid rights', async () => {
    const order = await paid();
    await db.query("insert into portal_test_grants(user_id,expires_at) values($1,'2026-10-10T15:00:00Z')", [user]);
    expect(await usage('reserve', crypto.randomUUID(), user, 120, true, [order])).toMatchObject({ allowed: true, plan: 'ADMIN', remaining_seconds: 1800 });
    expect((await db.query<{
        activated_at: string | null;
    }>('select activated_at from portal_entitlements')).rows[0].activated_at).toBeNull();
    expect((await db.query('select * from portal_events')).rows).toHaveLength(0);
});
it('revoking Premium denies replay immediately', async () => {
    const order = await paid(), id = crypto.randomUUID();
    await usage('reserve', id, user, 120, true, [order]);
    await db.exec('update portal_entitlements set revoked_at=public.test_now()');
    expect((await usage('reserve', id, user, 120, true, [order])).allowed).toBe(false);
});
it('validates request bounds and rejects completion of unknown UUID', async () => {
    for (const max of [0, 121, -1])
        await expect(usage('reserve', crypto.randomUUID(), user, max)).rejects.toThrow();
    await expect(usage('bogus')).rejects.toThrow();
    expect(await usage('complete')).toMatchObject({ allowed: false, error_code: 'OPERATION_NOT_FOUND' });
});
it('restricts RPC and ledger access to service_role with RLS', async () => {
    const p = (await db.query(`select has_function_privilege('authenticated','public.portal_usage(uuid,uuid,text,integer,boolean,uuid[])','EXECUTE') rpc,has_function_privilege('anon','public.portal_usage_status(uuid)','EXECUTE') status,has_table_privilege('authenticated','portal_usage_operations','INSERT') writes,(select relrowsecurity from pg_class where oid='portal_usage_operations'::regclass) rls`)).rows[0];
    expect(p).toEqual({ rpc: false, status: false, writes: false, rls: true });
    await db.exec('set role authenticated');
    await expect(db.query('select portal_usage_status($1::uuid)', [user])).rejects.toThrow(/permission denied/);
    await db.exec('reset role');
});
it('production SQL has no test clock or client elapsed-time override', () => {
    expect(readFileSync(new URL('../supabase/migrations/202610090003_daily_free.sql', import.meta.url), 'utf8')).not.toMatch(/test_now|test_clock|p_elapsed|p_now/);
});
it('pending verified Premium remains dispatchable after Free quota exhaustion', async () => {
    for (let i = 0; i < 15; i++) {
        await time(new Date(Date.parse(initial) + i * 120000).toISOString());
        await usage();
    }
    await time('2026-10-09T15:30:00Z');
    const order = await paid();
    expect(await usage('inspect', crypto.randomUUID(), user, 120, true, [order])).toMatchObject({ allowed: true, plan: 'PREMIUM', remaining_seconds: 0 });
    expect((await status()).plan).toBe('FREE');
    expect((await db.query<{
        activated_at: string | null;
    }>('select activated_at from portal_entitlements')).rows[0].activated_at).toBeNull();
});
it('fractional execution is conservatively rounded across the interval union', async () => {
    const id = crypto.randomUUID();
    await usage('reserve', id);
    await time('2026-10-09T15:00:00.001Z');
    expect((await usage('complete', id)).remaining_seconds).toBe(1799);
    await time('2026-10-09T15:00:01Z');
    const next = crypto.randomUUID();
    await usage('reserve', next);
    await time('2026-10-09T15:00:01.001Z');
    expect((await usage('complete', next)).remaining_seconds).toBe(1799);
});
it('completion and reserve replays cannot prolong the logical idle session', async () => {
    const id = crypto.randomUUID();
    await usage('reserve', id);
    await time('2026-10-09T15:00:30Z');
    const completed = await usage('complete', id);
    await time('2026-10-09T15:01:00Z');
    expect((await usage('complete', id)).session_active_until).toBe(completed.session_active_until);
    expect((await usage('reserve', id)).session_active_until).toBe(completed.session_active_until);
    await time('2026-10-09T15:02:30Z');
    expect((await status()).session_active_until).toBeNull();
});
it('expired Admin falls back to Free and cannot replay its old grant', async () => {
    await db.query("insert into portal_test_grants(user_id,expires_at) values($1,'2026-10-09T15:00:10Z')", [user]);
    const id = crypto.randomUUID();
    expect((await usage('reserve', id)).expires_at).toBe(epoch(initial) + 10);
    await time('2026-10-09T15:00:10Z');
    expect((await usage('reserve', id)).allowed).toBe(false);
    expect(await usage()).toMatchObject({ plan: 'FREE', remaining_seconds: 1680 });
});
it('legacy operation IDs cannot resurrect as new Free reservations', async () => {
    const id = crypto.randomUUID();
    await db.query('insert into portal_operations(user_id,operation_id,entitlement_id) values($1,$2,null)', [user,id]);
    expect(await usage('reserve',id)).toMatchObject({allowed:false,error_code:'OPERATION_ID_CONFLICT'});
    expect((await status()).remaining_seconds).toBe(1800);
    expect((await db.query('select * from portal_usage_operations')).rows).toHaveLength(0);
});

it('Premium activation stops Free billing at the activation instant', async () => {
    const free = crypto.randomUUID();await usage('reserve',free);
    await time('2026-10-09T15:00:25Z');const order=await paid();
    await usage('reserve',crypto.randomUUID(),user,120,true,[order]);
    await time('2026-10-09T15:00:50Z');
    expect((await usage('complete',free)).remaining_seconds).toBe(1775);
    const row=(await db.query<{plan:string;completed_at:string}>('select plan,completed_at from portal_usage_operations where operation_id=$1',[free])).rows[0];
    expect(row.plan).toBe('FREE');
    expect(epoch(row.completed_at)).toBe(epoch(initial)+50);
});
it('Admin grant stops Free billing without rewriting the original execution plan', async () => {
    const free=crypto.randomUUID();await usage('reserve',free);
    await time('2026-10-09T15:00:25Z');
    await db.query("insert into portal_test_grants(user_id,expires_at) values($1,'2026-10-10T15:00:00Z')",[user]);
    await usage('inspect');await time('2026-10-09T15:00:50Z');
    expect((await usage('complete',free)).remaining_seconds).toBe(1775);
});
it('pending Premium inspect does not refund a running Free reservation', async () => {
    const free=crypto.randomUUID();await usage('reserve',free);
    await time('2026-10-09T15:00:25Z');const order=await paid();
    await usage('inspect',crypto.randomUUID(),user,120,true,[order]);
    await time('2026-10-09T15:00:50Z');
    expect((await usage('complete',free)).remaining_seconds).toBe(1750);
});
it('abort refunds only an unfinished server-proven undispatched operation and tombstones its UUID', async () => {
    const id=crypto.randomUUID();await usage('reserve',id);
    await time('2026-10-09T15:00:25Z');
    expect(await usage('abort',id)).toMatchObject({allowed:false,state:'ABORTED',remaining_seconds:1800});
    expect(await usage('reserve',id)).toMatchObject({allowed:false,state:'ABORTED'});
    expect(await usage('complete',id)).toMatchObject({allowed:false,state:'ABORTED',remaining_seconds:1800});
    const row=(await db.query<{aborted:boolean;elapsed:number}>('select aborted,extract(epoch from completed_at-started_at)::int elapsed from portal_usage_operations where operation_id=$1',[id])).rows[0];
    expect(row).toEqual({aborted:true,elapsed:0});
});
it('abort cannot refund a previously completed operation', async () => {
    const id=crypto.randomUUID();await usage('reserve',id);
    await time('2026-10-09T15:00:25Z');await usage('complete',id);
    expect(await usage('abort',id)).toMatchObject({allowed:false,state:'COMPLETED',remaining_seconds:1775});
});
it('relative lease milliseconds use SQL time and diminish on replay at midnight', async () => {
    await time('2026-10-10T02:59:50Z');const id=crypto.randomUUID();
    expect(await usage('reserve',id)).toMatchObject({lease_remaining_ms:10000});
    await time('2026-10-10T02:59:52Z');expect(await usage('reserve',id)).toMatchObject({lease_remaining_ms:8000});
    expect(await usage('complete',id)).toMatchObject({lease_remaining_ms:0});
});
it('revoked Premium exempts only its historically valid window during a Free execution', async () => {
    const free=crypto.randomUUID();await usage('reserve',free);
    await time('2026-10-09T15:00:30Z');const order=await paid();
    await usage('reserve',crypto.randomUUID(),user,120,true,[order]);
    await time('2026-10-09T15:00:40Z');
    await db.exec('update portal_entitlements set revoked_at=public.test_now()');
    await time('2026-10-09T15:02:00Z');
    expect((await usage('complete',free)).remaining_seconds).toBe(1690);
});
it('Admin expiry resumes Free billing within the original operation interval', async () => {
    const free=crypto.randomUUID();await usage('reserve',free);
    await time('2026-10-09T15:00:30Z');
    await db.query("insert into portal_test_grants(user_id,expires_at) values($1,'2026-10-09T15:00:40Z')",[user]);
    await usage('inspect');
    await time('2026-10-09T15:02:00Z');
    expect((await usage('complete',free)).remaining_seconds).toBe(1690);
});
