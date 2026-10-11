-- Supabase Auth is supplied by the host. No secrets or provider I/O in this migration.
create table public.portal_customers (
 user_id uuid primary key references auth.users(id), provider_id text unique,
 creation_state text not null default 'RESERVED' check(creation_state in ('RESERVED','CREATING','READY','UNKNOWN')),
 created_at timestamptz not null default now()
);
create table public.portal_orders (
 id uuid primary key default gen_random_uuid(), user_id uuid not null references auth.users(id),
 amount_cents bigint not null check(amount_cents>0 and amount_cents<=100000000),
 status text not null default 'PENDING_PAYMENT' check(status in ('PENDING_PAYMENT','PAID_AWAITING_SERVICE','READY_TO_ACTIVATE','ACTIVE','EXPIRED','CANCELED','REFUNDED')),
 creation_state text not null default 'RESERVED' check(creation_state in ('RESERVED','CREATING','READY','UNKNOWN')),
 customer_id text, payment_id text unique, payment_url text, refund_state text check(refund_state in ('REQUESTED','CREATING','UNKNOWN','CONFIRMED')),
 created_at timestamptz not null default now(), updated_at timestamptz not null default now(), terms_accepted_at timestamptz not null default now()
);
create unique index portal_order_pending on public.portal_orders(user_id) where status='PENDING_PAYMENT';
create table public.portal_entitlements (
 id uuid primary key default gen_random_uuid(), user_id uuid not null references auth.users(id), order_id uuid not null unique references public.portal_orders(id),
 activated_at timestamptz, expires_at timestamptz, revoked_at timestamptz,
 created_at timestamptz not null default now(),
 check ((activated_at is null and expires_at is null) or (activated_at is not null and expires_at=activated_at+interval '2592000 seconds'))
);
create table public.portal_events (
 id bigint generated always as identity primary key, user_id uuid not null references auth.users(id), order_id uuid references public.portal_orders(id),
 event_key text not null unique, kind text not null, created_at timestamptz not null default now()
);
create table public.portal_operations (
 user_id uuid not null references auth.users(id), operation_id uuid not null,
 entitlement_id uuid references public.portal_entitlements(id), created_at timestamptz not null default now(), primary key(user_id,operation_id)
);
create table public.portal_tickets (
 ticket_hash text primary key, user_id uuid not null references auth.users(id), email text not null,
 state text not null, audience text not null, expires_at timestamptz not null default now()+interval '60 seconds', consumed_at timestamptz
);
create table public.portal_rate_limits (key text primary key, starts_at timestamptz not null, hits integer not null);

alter table public.portal_customers enable row level security;
alter table public.portal_orders enable row level security;
alter table public.portal_entitlements enable row level security;
alter table public.portal_events enable row level security;
alter table public.portal_operations enable row level security;
alter table public.portal_tickets enable row level security;
alter table public.portal_rate_limits enable row level security;
create policy own_orders on public.portal_orders for select to authenticated using(user_id=auth.uid());
create policy own_entitlements on public.portal_entitlements for select to authenticated using(user_id=auth.uid());
create policy own_events on public.portal_events for select to authenticated using(user_id=auth.uid());
revoke all on public.portal_customers,public.portal_orders,public.portal_entitlements,public.portal_events,public.portal_operations,public.portal_tickets,public.portal_rate_limits from public,anon,authenticated;
grant select on public.portal_orders, public.portal_entitlements, public.portal_events to authenticated;
grant all on public.portal_customers,public.portal_orders,public.portal_entitlements,public.portal_events,public.portal_operations,public.portal_tickets,public.portal_rate_limits to service_role;
grant usage,select on sequence public.portal_events_id_seq to service_role;

create function public.portal_subscription(p_user uuid,p_available boolean) returns jsonb language plpgsql security definer set search_path=public,pg_temp as $$
declare e portal_entitlements; s text; n integer; o portal_orders;
begin
 select * into e from portal_entitlements where user_id=p_user and revoked_at is null and expires_at>now() order by activated_at limit 1;
 select count(*)*30 into n from portal_entitlements where user_id=p_user and revoked_at is null and activated_at is null;
 if e.id is not null then s:='ACTIVE';
 elsif n>0 then s:=case when p_available then 'READY_TO_ACTIVATE' else 'PAID_AWAITING_SERVICE' end;
 else
 select * into o from portal_orders where user_id=p_user order by created_at desc limit 1;
 s:=coalesce(o.status,'PENDING_PAYMENT');
 if exists(select 1 from portal_entitlements where user_id=p_user and revoked_at is null and expires_at<=now()) then s:='EXPIRED'; end if;
 end if;
 return jsonb_build_object('state',s,'pendingDays',n,'activatedAt',e.activated_at,'expiresAt',e.expires_at);
end $$;

create function public.portal_checkout_reserve(p_user uuid,p_amount bigint) returns jsonb language plpgsql security definer set search_path=public,pg_temp as $$
declare o portal_orders;
begin
 perform pg_advisory_xact_lock(hashtextextended(p_user::text,0));
 insert into portal_customers(user_id) values(p_user) on conflict do nothing;
 select * into o from portal_orders where user_id=p_user and status='PENDING_PAYMENT' order by created_at limit 1;
 if o.id is null then insert into portal_orders(user_id,amount_cents) values(p_user,p_amount) returning * into o; end if;
 return to_jsonb(o);
end $$;
create function public.portal_checkout_complete(p_order uuid,p_customer text,p_payment text,p_url text) returns void language plpgsql security definer set search_path=public,pg_temp as $$
begin
 update portal_orders set customer_id=p_customer,payment_id=p_payment,payment_url=p_url,creation_state='READY',updated_at=now() where id=p_order and (payment_id is null or payment_id=p_payment);
 if not found then raise exception 'payment conflict'; end if;
end $$;

create function public.portal_payment_apply(p_order uuid,p_event text,p_status text) returns jsonb language plpgsql security definer set search_path=public,pg_temp as $$
declare o portal_orders;
begin
 select * into o from portal_orders where id=p_order;
 if o.id is null then raise exception 'unknown order'; end if;
 perform pg_advisory_xact_lock(hashtextextended(o.user_id::text,0));
 select * into o from portal_orders where id=p_order for update;
 insert into portal_events(user_id,order_id,event_key,kind) values(o.user_id,o.id,p_event,p_status) on conflict do nothing;
 if p_status in ('REFUNDED','REFUND_PARTIALLY_DONE') then
 update portal_orders set status='REFUNDED',refund_state='CONFIRMED',updated_at=now() where id=o.id;
 update portal_entitlements set revoked_at=coalesce(revoked_at,now()) where order_id=o.id;
 elsif p_status='RECEIVED' and o.status<>'REFUNDED' then
 insert into portal_entitlements(user_id,order_id) values(o.user_id,o.id) on conflict(order_id) do nothing;
 update portal_orders set status='PAID_AWAITING_SERVICE',updated_at=now() where id=o.id and status in ('PENDING_PAYMENT','CANCELED');
 elsif p_status in ('DELETED','CANCELED') and o.status='PENDING_PAYMENT' then
 update portal_orders set status='CANCELED',updated_at=now() where id=o.id;
 end if;
 return (select to_jsonb(x) from portal_orders x where x.id=o.id);
end $$;

create function public.portal_authorize(p_user uuid,p_operation uuid,p_mode text,p_available boolean) returns jsonb language plpgsql security definer set search_path=public,pg_temp as $$
declare e portal_entitlements; op portal_operations; s jsonb; permitted boolean:=false;
begin
 if p_mode not in ('inspect','execute') then raise exception 'invalid mode'; end if;
 perform pg_advisory_xact_lock(hashtextextended(p_user::text,0));
 s:=portal_subscription(p_user,p_available);
 if p_mode='execute' then
 select * into op from portal_operations where user_id=p_user and operation_id=p_operation;
 if found then
 select * into e from portal_entitlements where id=op.entitlement_id;
 elsif p_available then
 select * into e from portal_entitlements where user_id=p_user and revoked_at is null and expires_at>now() order by activated_at limit 1;
 if e.id is null then
 select * into e from portal_entitlements where user_id=p_user and revoked_at is null and activated_at is null order by created_at,id limit 1 for update;
 if e.id is not null then
 update portal_entitlements set activated_at=now(),expires_at=now()+interval '2592000 seconds' where id=e.id returning * into e;
 update portal_orders set status='ACTIVE',updated_at=now() where id=e.order_id;
 insert into portal_events(user_id,order_id,event_key,kind) values(p_user,e.order_id,'activation:'||e.id,'ACTIVATED');
 end if;
 end if;
 -- Audit every admitted execution, including a denied execution. A retry never acquires another credit.
 insert into portal_operations(user_id,operation_id,entitlement_id) values(p_user,p_operation,e.id);
 end if;
 else
 select * into e from portal_entitlements where user_id=p_user and revoked_at is null and expires_at>now() order by activated_at limit 1;
 end if;
 permitted:=p_available and e.id is not null and e.revoked_at is null and e.expires_at>now();
 s:=portal_subscription(p_user,p_available);
 return jsonb_build_object('subject',p_user,'operation_id',p_operation,'allowed',permitted,'state',s->>'state','expires_at',case when e.id is not null then floor(extract(epoch from e.expires_at)) else null end);
end $$;

create function public.portal_ticket_create(p_hash text,p_user uuid,p_email text,p_state text,p_audience text) returns void language sql security definer set search_path=public,pg_temp as $$
 insert into portal_tickets(ticket_hash,user_id,email,state,audience) values(p_hash,p_user,p_email,p_state,p_audience);
$$;
create function public.portal_ticket_exchange(p_hash text,p_state text,p_audience text) returns jsonb language plpgsql security definer set search_path=public,pg_temp as $$
declare t portal_tickets;
begin
 update portal_tickets set consumed_at=now() where ticket_hash=p_hash and state=p_state and audience=p_audience and expires_at>now() and consumed_at is null returning * into t;
 if t.ticket_hash is null then return null; end if;
 return jsonb_build_object('subject',t.user_id,'email',t.email,'email_verified',true,'audience',t.audience);
end $$;
create function public.portal_rate_limit(p_key text,p_limit integer,p_seconds integer) returns boolean language plpgsql security definer set search_path=public,pg_temp as $$
declare n integer;
begin
 if p_limit<1 or p_seconds<1 then raise exception 'invalid limit'; end if;
 insert into portal_rate_limits as r(key,starts_at,hits) values(p_key,now(),1) on conflict(key) do update set starts_at=case when r.starts_at+make_interval(secs=>p_seconds)<=now() then now() else r.starts_at end,hits=case when r.starts_at+make_interval(secs=>p_seconds)<=now() then 1 else r.hits+1 end returning hits into n;
 return n<=p_limit;
end $$;
-- PostgreSQL grants execute to PUBLIC by default: revoke each authority function explicitly.
revoke all on function public.portal_subscription(uuid,boolean),public.portal_checkout_reserve(uuid,bigint),public.portal_checkout_complete(uuid,text,text,text),public.portal_payment_apply(uuid,text,text),public.portal_authorize(uuid,uuid,text,boolean),public.portal_ticket_create(text,uuid,text,text,text),public.portal_ticket_exchange(text,text,text),public.portal_rate_limit(text,integer,integer) from public,anon,authenticated;
grant execute on function public.portal_subscription(uuid,boolean),public.portal_checkout_reserve(uuid,bigint),public.portal_checkout_complete(uuid,text,text,text),public.portal_payment_apply(uuid,text,text),public.portal_authorize(uuid,uuid,text,boolean),public.portal_ticket_create(text,uuid,text,text,text),public.portal_ticket_exchange(text,text,text),public.portal_rate_limit(text,integer,integer) to service_role;
