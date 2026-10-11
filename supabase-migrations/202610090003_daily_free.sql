-- Additive Free quota ledger. The database clock alone controls leases and billing.
begin;
set local lock_timeout = '5s';
set local statement_timeout = '30s';

create table public.portal_usage_operations (
 operation_id uuid primary key,
 user_id uuid not null references auth.users(id),
 plan text not null check (plan in ('FREE','PREMIUM','ADMIN')),
 started_at timestamptz not null,
 deadline timestamptz not null,
 completed_at timestamptz,
 aborted boolean not null default false,
 entitlement_id uuid references public.portal_entitlements(id),
 unique (user_id,operation_id),
 check (deadline > started_at and deadline <= started_at + interval '120 seconds'),
 check (completed_at is null or (completed_at >= started_at and completed_at <= deadline)),
 check (not aborted or (completed_at is not null and completed_at=started_at))
);
create index portal_usage_user_interval on public.portal_usage_operations(user_id,started_at,deadline) where plan='FREE';
create table public.portal_usage_sessions (
 user_id uuid primary key references auth.users(id),
 last_activity_at timestamptz not null
);
alter table public.portal_usage_operations enable row level security;
alter table public.portal_usage_sessions enable row level security;
revoke all on public.portal_usage_operations,public.portal_usage_sessions from public,anon,authenticated;
grant all on public.portal_usage_operations,public.portal_usage_sessions to service_role;

-- Union completed intervals and full pending reservations, clipped to one local day.
-- Pending rows keep their full lease after a crash; elapsed wall time cannot refund them.
-- Subtract only historically valid Premium/Admin windows. Revocation or expiry
-- resumes Free billing without rewriting the original execution's plan/lifecycle.
-- Admin grants have one row per account: only trusted administrators can delete
-- or rewrite it; deletion conservatively removes that historical exemption.
create function public.portal_free_seconds(p_user uuid,p_start timestamptz,p_end timestamptz,
 p_extra_start timestamptz default null,p_extra_end timestamptz default null) returns integer
language sql stable security definer set search_path=public,pg_temp as $$
 with intervals as (
  select greatest(started_at,p_start) a,least(coalesce(completed_at,deadline),p_end) b
  from portal_usage_operations where user_id=p_user and plan='FREE'
   and started_at>=p_start-interval '120 seconds' and started_at<p_end
   and coalesce(completed_at,deadline)>p_start
  union all select greatest(p_extra_start,p_start),least(p_extra_end,p_end)
   where p_extra_start is not null and p_extra_end is not null and p_extra_end>p_extra_start
 ), exemptions as (
  select greatest(activated_at,p_start) a,
   least(expires_at,revoked_at,p_end) b
  from portal_entitlements where user_id=p_user and activated_at is not null
   and activated_at<p_end and least(expires_at,revoked_at)>p_start
  union all
  select greatest(granted_at,p_start),least(expires_at,p_end)
  from portal_test_grants where user_id=p_user and granted_at<p_end and expires_at>p_start
 ), merged as (
  select coalesce((select range_agg(tstzrange(a,b,'[)')) from intervals where b>a),'{}'::tstzmultirange)
   - coalesce((select range_agg(tstzrange(a,b,'[)')) from exemptions where b>a),'{}'::tstzmultirange) ranges
 )
 select coalesce(ceil(sum(extract(epoch from upper(r)-lower(r)))),0)::integer
 from merged cross join lateral unnest(ranges) r;
$$;
revoke all on function public.portal_free_seconds(uuid,timestamptz,timestamptz,timestamptz,timestamptz) from public,anon,authenticated;
grant execute on function public.portal_free_seconds(uuid,timestamptz,timestamptz,timestamptz,timestamptz) to service_role;

-- Read-only UI view. Pending paid rights are intentionally never activated here.
create function public.portal_usage_status(p_user uuid) returns jsonb
language plpgsql volatile security definer set search_path=public,pg_temp as $$
declare t timestamptz:=clock_timestamp(); d timestamptz; reset_at timestamptz; expiry timestamptz;
 plan_name text:='FREE'; used integer; activity timestamptz;
begin
 if p_user is null then raise exception 'user is required' using errcode='22004'; end if;
 d:=(t at time zone 'America/Sao_Paulo')::date::timestamp at time zone 'America/Sao_Paulo';
 reset_at:=((t at time zone 'America/Sao_Paulo')::date+1)::timestamp at time zone 'America/Sao_Paulo';
 select expires_at into expiry from portal_test_grants where user_id=p_user and expires_at>t;
 if expiry is not null then plan_name:='ADMIN'; else
  select expires_at into expiry from portal_entitlements where user_id=p_user and revoked_at is null and expires_at>t order by activated_at limit 1;
  if expiry is not null then plan_name:='PREMIUM'; end if;
 end if;
 used:=portal_free_seconds(p_user,d,reset_at);
 select least(last_activity_at+interval '120 seconds',reset_at) into activity from portal_usage_sessions where user_id=p_user;
 if activity<=t then activity:=null; end if;
 return jsonb_build_object('subject',p_user,'operation_id',null,'allowed',plan_name<>'FREE' or used<1800,
  'state',case when plan_name='FREE' and used>=1800 then 'QUOTA_EXHAUSTED' else 'ACTIVE' end,
  'plan',plan_name,'expires_at',floor(extract(epoch from expiry)),
  'remaining_seconds',greatest(0,1800-used),'resets_at',floor(extract(epoch from reset_at)),
  'error_code',null,'session_active_until',floor(extract(epoch from activity)));
end $$;
revoke all on function public.portal_usage_status(uuid) from public,anon,authenticated;
grant execute on function public.portal_usage_status(uuid) to service_role;

create function public.portal_usage(p_user uuid,p_operation uuid,p_action text,p_max_seconds integer,
 p_available boolean,p_verified_orders uuid[]) returns jsonb
language plpgsql security definer set search_path=public,pg_temp as $$
declare
 t timestamptz; d timestamptz; reset_at timestamptz; expiry timestamptz; until_at timestamptz;
 op portal_usage_operations; ent portal_entitlements; tg portal_test_grants; auth_result jsonb; result jsonb;
 plan_name text:='FREE'; permitted boolean:=false; state_name text:='ACTIVE'; code text;
 used integer; lo integer; hi integer; mid integer;
begin
 if p_user is null or p_operation is null or p_verified_orders is null or p_available is null then
  raise exception 'user, operation, verified orders and availability are required' using errcode='22004';
 end if;
 if p_action is null or p_action not in ('inspect','reserve','complete','abort') or p_max_seconds is null or p_max_seconds not between 1 and 120 then
  raise exception 'invalid usage action or lease bounds' using errcode='22023';
 end if;
 -- Both identity and quota are serialized. Authorization uses this same account lock.
 perform pg_advisory_xact_lock(hashtextextended(p_operation::text,1));
 perform pg_advisory_xact_lock(hashtextextended(p_user::text,0));
 t:=clock_timestamp();
 d:=(t at time zone 'America/Sao_Paulo')::date::timestamp at time zone 'America/Sao_Paulo';
 reset_at:=((t at time zone 'America/Sao_Paulo')::date+1)::timestamp at time zone 'America/Sao_Paulo';
 select * into op from portal_usage_operations where operation_id=p_operation;
 if (op.operation_id is not null and op.user_id<>p_user) or exists(select 1 from portal_operations where operation_id=p_operation and (user_id<>p_user or op.operation_id is null)) then
  result:=portal_usage_status(p_user);
  return result || jsonb_build_object('operation_id',p_operation,'allowed',false,'state','DENIED','error_code','OPERATION_ID_CONFLICT','lease_remaining_ms',0);
 end if;
 if p_action in ('complete','abort') then
  if op.operation_id is null then
   result:=portal_usage_status(p_user);
   return result || jsonb_build_object('operation_id',p_operation,'allowed',false,'state','DENIED','error_code','OPERATION_NOT_FOUND','lease_remaining_ms',0);
  end if;
  -- Abort is a trusted-server assertion that dispatch NEVER occurred. The service
  -- must hold its exclusive dispatch claim before using it. No client may call RPC.
  if p_action='abort' and op.completed_at is null then
   update portal_usage_operations set completed_at=started_at,aborted=true
    where operation_id=p_operation returning * into op;
  end if;
  if op.completed_at is null then
   update portal_usage_operations set completed_at=greatest(started_at,least(t,deadline)) where operation_id=p_operation returning * into op;
   insert into portal_usage_sessions(user_id,last_activity_at) values(p_user,op.completed_at)
    on conflict(user_id) do update set last_activity_at=greatest(portal_usage_sessions.last_activity_at,excluded.last_activity_at);
  end if;
  result:=portal_usage_status(p_user);
  return result || jsonb_build_object('operation_id',p_operation,'allowed',false,
   'state',case when op.aborted then 'ABORTED' else 'COMPLETED' end,
   'expires_at',floor(extract(epoch from op.deadline)),'lease_remaining_ms',0);
 end if;

 select * into tg from portal_test_grants where user_id=p_user and expires_at>t;
 if tg.user_id is not null then plan_name:='ADMIN';expiry:=tg.expires_at; else
  -- Billing has reconciled the supplied list. Inspect is read-only; only reserve activates.
  if p_action='reserve' and op.operation_id is null and p_available and exists(
   select 1 from portal_entitlements where user_id=p_user and revoked_at is null
    and (activated_at is null or expires_at>t) and order_id=any(p_verified_orders)) then
   auth_result:=portal_authorize(p_user,p_operation,'execute',p_available,p_verified_orders);
  end if;
  select * into ent from portal_entitlements where user_id=p_user and revoked_at is null
   and (expires_at>t or (p_action='inspect' and activated_at is null))
   and order_id=any(p_verified_orders) order by activated_at nulls last,created_at,id limit 1;
  if ent.id is not null then
   plan_name:='PREMIUM';expiry:=ent.expires_at;
  end if;
 end if;
 used:=portal_free_seconds(p_user,d,reset_at);
 until_at:=least(t+make_interval(secs=>p_max_seconds),reset_at,coalesce(expiry,reset_at));
 if op.operation_id is not null then
  until_at:=op.deadline;
  if op.aborted then state_name:='ABORTED';code:='OPERATION_ABORTED';
  elsif op.completed_at is not null then state_name:='COMPLETED';code:='OPERATION_COMPLETED';
  elsif op.deadline<=t then state_name:='EXPIRED';code:='OPERATION_EXPIRED';
  elsif op.plan='PREMIUM' and (plan_name<>'PREMIUM' or ent.id is distinct from op.entitlement_id) then state_name:='DENIED';code:='ENTITLEMENT_REVOKED';
  elsif op.plan='ADMIN' and plan_name<>'ADMIN' then state_name:='DENIED';code:='ENTITLEMENT_REVOKED';
  elsif not p_available then state_name:='UNAVAILABLE';code:='SERVICE_UNAVAILABLE';
  else permitted:=true; end if;
 elsif not p_available then state_name:='UNAVAILABLE';code:='SERVICE_UNAVAILABLE';
 elsif p_action='inspect' then
  permitted:=plan_name<>'FREE' or used<1800;
  if not permitted then state_name:='QUOTA_EXHAUSTED';code:='FREE_DAILY_LIMIT_REACHED'; end if;
 else
  if plan_name='FREE' then
   -- Binary search the largest integral-second lease whose union fits the cap.
   -- This permits overlap even if the entire budget is already reserved.
   lo:=0;hi:=greatest(0,floor(extract(epoch from until_at-t))::integer);
   while lo<hi loop
    mid:=(lo+hi+1)/2;
    if portal_free_seconds(p_user,d,reset_at,t,t+make_interval(secs=>mid))<=1800 then lo:=mid; else hi:=mid-1; end if;
   end loop;
   until_at:=t+make_interval(secs=>lo);
  end if;
  if until_at<=t then state_name:='QUOTA_EXHAUSTED';code:='FREE_DAILY_LIMIT_REACHED'; else
   permitted:=true;
   insert into portal_usage_operations(operation_id,user_id,plan,started_at,deadline,entitlement_id)
    values(p_operation,p_user,plan_name,t,until_at,ent.id);
   insert into portal_usage_sessions(user_id,last_activity_at) values(p_user,t)
    on conflict(user_id) do update set last_activity_at=greatest(portal_usage_sessions.last_activity_at,excluded.last_activity_at);
  end if;
 end if;
 result:=portal_usage_status(p_user);
 return result || jsonb_build_object('operation_id',p_operation,'allowed',permitted,'state',state_name,'plan',plan_name,
  'expires_at',floor(extract(epoch from until_at)),'error_code',code,
  'lease_remaining_ms',case when permitted then
   least(120000,greatest(0,floor(extract(epoch from until_at-clock_timestamp())*1000)))::integer else 0 end);
end $$;
revoke all on function public.portal_usage(uuid,uuid,text,integer,boolean,uuid[]) from public,anon,authenticated;
grant execute on function public.portal_usage(uuid,uuid,text,integer,boolean,uuid[]) to service_role;
notify pgrst, 'reload schema';
commit;
