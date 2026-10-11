-- Additive installation for a database with migration 202610080001 applied.
-- Preserve the existing four-argument function, its OID and its dependencies.
-- Five arguments are required: no DEFAULT, therefore no overload ambiguity.
-- Do not run only a selection: execute this entire BEGIN ... COMMIT script.
begin;
set local lock_timeout = '5s';
set local statement_timeout = '30s';

-- Read-only preconditions; unexpected state aborts the transaction.
do $preflight$
declare expected record; legacy oid; subscription_fn oid;
begin
 legacy := to_regprocedure('public.portal_authorize(uuid,uuid,text,boolean)');
 subscription_fn := to_regprocedure('public.portal_subscription(uuid,boolean)');
 if legacy is null or subscription_fn is null then
  raise exception 'Expected base portal functions are missing; stop and audit the database';
 end if;
 if to_regprocedure('public.portal_authorize(uuid,uuid,text,boolean,uuid[])') is not null then
  raise exception 'Five-argument authorization already exists; this additive script will not overwrite it';
 end if;
 if exists(select 1 from pg_proc where oid in (legacy,subscription_fn) and prorettype <> 'jsonb'::regtype) then
  raise exception 'Unexpected return type on base portal functions';
 end if;
 if to_regrole('anon') is null or to_regrole('authenticated') is null or to_regrole('service_role') is null then
  raise exception 'Expected Supabase roles are missing';
 end if;
 if has_function_privilege('anon',legacy,'EXECUTE') or has_function_privilege('authenticated',legacy,'EXECUTE')
    or not has_function_privilege('service_role',legacy,'EXECUTE') then
  raise exception 'Unexpected privileges on legacy authorization; stop and audit permissions';
 end if;
 for expected in select * from (values
  ('portal_entitlements','id','uuid'), ('portal_entitlements','user_id','uuid'),
  ('portal_entitlements','order_id','uuid'), ('portal_entitlements','revoked_at','timestamptz'),
  ('portal_entitlements','expires_at','timestamptz'), ('portal_entitlements','activated_at','timestamptz'),
  ('portal_entitlements','created_at','timestamptz'),
  ('portal_operations','user_id','uuid'), ('portal_operations','operation_id','uuid'),
  ('portal_operations','entitlement_id','uuid'),
  ('portal_orders','id','uuid'), ('portal_orders','status','text'), ('portal_orders','updated_at','timestamptz'),
  ('portal_events','user_id','uuid'), ('portal_events','order_id','uuid'),
  ('portal_events','event_key','text'), ('portal_events','kind','text')
 ) as required(table_name,column_name,type_name) loop
  if not exists(select 1 from pg_attribute a join pg_class c on c.oid=a.attrelid
    join pg_namespace n on n.oid=c.relnamespace
    where n.nspname='public' and c.relname=expected.table_name and c.relkind='r'
      and c.relrowsecurity and a.attname=expected.column_name and not a.attisdropped
      and a.atttypid=to_regtype(expected.type_name)) then
   raise exception 'Missing or incompatible column/RLS: public.%.%',expected.table_name,expected.column_name;
  end if;
 end loop;
end $preflight$;

create function public.portal_authorize(p_user uuid,p_operation uuid,p_mode text,p_available boolean,p_verified_orders uuid[]) returns jsonb
language plpgsql security definer set search_path=public,pg_temp as $$
declare e portal_entitlements; op portal_operations; s jsonb; permitted boolean:=false;
begin
 if p_verified_orders is null then raise exception 'Verified order list is required' using errcode='22004'; end if;
 if p_mode not in ('inspect','execute') then raise exception 'invalid mode'; end if;
 perform pg_advisory_xact_lock(hashtextextended(p_user::text,0));
 if p_mode='execute' then
  select * into op from portal_operations where user_id=p_user and operation_id=p_operation;
  if found then
   select * into e from portal_entitlements where id=op.entitlement_id and user_id=p_user
    and order_id=any(p_verified_orders);
  elsif p_available then
   select * into e from portal_entitlements where user_id=p_user and revoked_at is null and expires_at>now()
    and order_id=any(p_verified_orders) order by activated_at limit 1;
   if e.id is null then
    select * into e from portal_entitlements where user_id=p_user and revoked_at is null and activated_at is null
     and order_id=any(p_verified_orders) order by created_at,id limit 1 for update;
    if e.id is not null then
     update portal_entitlements set activated_at=now(),expires_at=now()+interval '2592000 seconds' where id=e.id returning * into e;
     update portal_orders set status='ACTIVE',updated_at=now() where id=e.order_id;
     insert into portal_events(user_id,order_id,event_key,kind) values(p_user,e.order_id,'activation:'||e.id,'ACTIVATED');
    end if;
   end if;
   insert into portal_operations(user_id,operation_id,entitlement_id) values(p_user,p_operation,e.id);
  end if;
 else
  select * into e from portal_entitlements where user_id=p_user and revoked_at is null and expires_at>now()
   and order_id=any(p_verified_orders) order by activated_at limit 1;
 end if;
 permitted:=p_available and e.id is not null and e.revoked_at is null and e.expires_at>now();
 s:=portal_subscription(p_user,p_available);
 return jsonb_build_object('subject',p_user,'operation_id',p_operation,'allowed',permitted,'state',s->>'state',
  'expires_at',case when e.id is not null then floor(extract(epoch from e.expires_at)) else null end);
end $$;
revoke all on function public.portal_authorize(uuid,uuid,text,boolean,uuid[]) from public,anon,authenticated;
grant execute on function public.portal_authorize(uuid,uuid,text,boolean,uuid[]) to service_role;
notify pgrst, 'reload schema';
commit;
