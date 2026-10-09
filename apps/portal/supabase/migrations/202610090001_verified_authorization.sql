-- Restrict authorization to the exact orders just reconciled by the server.
-- Four-argument compatibility is retained for trusted service_role callers;
-- the HTTP authorization handler always supplies p_verified_orders, even [].
begin;
drop function public.portal_authorize(uuid,uuid,text,boolean);
create function public.portal_authorize(p_user uuid,p_operation uuid,p_mode text,p_available boolean,p_verified_orders uuid[] default null) returns jsonb
language plpgsql security definer set search_path=public,pg_temp as $$
declare e portal_entitlements; op portal_operations; s jsonb; permitted boolean:=false;
begin
 if p_mode not in ('inspect','execute') then raise exception 'invalid mode'; end if;
 perform pg_advisory_xact_lock(hashtextextended(p_user::text,0));
 if p_mode='execute' then
  select * into op from portal_operations where user_id=p_user and operation_id=p_operation;
  if found then
   select * into e from portal_entitlements where id=op.entitlement_id and user_id=p_user
    and (p_verified_orders is null or order_id=any(p_verified_orders));
  elsif p_available then
   select * into e from portal_entitlements where user_id=p_user and revoked_at is null and expires_at>now()
    and (p_verified_orders is null or order_id=any(p_verified_orders)) order by activated_at limit 1;
   if e.id is null then
    select * into e from portal_entitlements where user_id=p_user and revoked_at is null and activated_at is null
     and (p_verified_orders is null or order_id=any(p_verified_orders)) order by created_at,id limit 1 for update;
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
   and (p_verified_orders is null or order_id=any(p_verified_orders)) order by activated_at limit 1;
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
