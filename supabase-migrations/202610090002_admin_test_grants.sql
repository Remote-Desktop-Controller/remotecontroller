-- Migration 202610090002_admin_test_grants.sql
-- Concessões de teste administrativo: autorização comercial real sem falsificar pedidos pagos ou eventos do Asaas.
-- Não consome nem ativa os 30 dias de assinaturas pagas.

begin;
set local lock_timeout = '5s';
set local statement_timeout = '30s';

-- 1. Tabela isolada para concessões de teste administrativo
create table if not exists public.portal_test_grants (
  user_id uuid primary key references auth.users(id) on delete cascade,
  label text not null default 'TESTE_ADMINISTRATIVO',
  granted_by text not null default 'admin',
  granted_at timestamptz not null default now(),
  expires_at timestamptz not null default (now() + interval '30 days')
);

alter table public.portal_test_grants enable row level security;

do $$
begin
  if not exists (
    select 1 from pg_policies where schemaname = 'public' and tablename = 'portal_test_grants' and policyname = 'own_test_grants'
  ) then
    create policy own_test_grants on public.portal_test_grants for select to authenticated using (user_id = auth.uid());
  end if;
end $$;

revoke all on public.portal_test_grants from public, anon, authenticated;
grant select on public.portal_test_grants to authenticated;
grant all on public.portal_test_grants to service_role;

-- Funcao RPC para checagem rapida de concessao ativa
create or replace function public.portal_has_test_grant(p_user uuid) returns boolean
language sql security definer set search_path=public,pg_temp as $$
  select exists(select 1 from public.portal_test_grants where user_id = p_user and expires_at > now());
$$;
revoke all on function public.portal_has_test_grant(uuid) from public,anon,authenticated;
grant execute on function public.portal_has_test_grant(uuid) to service_role;

-- 2. Atualizar portal_subscription para reconhecer concessões de teste
create or replace function public.portal_subscription(p_user uuid, p_available boolean) returns jsonb
language plpgsql security definer set search_path=public,pg_temp as $$
declare
  tg portal_test_grants;
  e portal_entitlements;
  s text;
  n integer;
  o portal_orders;
begin
  -- Checar concessão de teste ativa primeiro
  select * into tg from portal_test_grants where user_id = p_user and expires_at > now();
  if tg.user_id is not null then
    s := case when p_available then 'ACTIVE' else 'PAID_AWAITING_SERVICE' end;
    select count(*)*30 into n from portal_entitlements where user_id = p_user and revoked_at is null and activated_at is null;
    return jsonb_build_object(
      'state', s,
      'pendingDays', n,
      'activatedAt', tg.granted_at,
      'expiresAt', tg.expires_at,
      'isTest', true,
      'testLabel', tg.label
    );
  end if;

  select * into e from portal_entitlements where user_id = p_user and revoked_at is null and expires_at > now() order by activated_at limit 1;
  select count(*)*30 into n from portal_entitlements where user_id = p_user and revoked_at is null and activated_at is null;
  if e.id is not null then s := 'ACTIVE';
  elsif n > 0 then s := case when p_available then 'READY_TO_ACTIVATE' else 'PAID_AWAITING_SERVICE' end;
  else
    select * into o from portal_orders where user_id = p_user order by created_at desc limit 1;
    s := coalesce(o.status, 'PENDING_PAYMENT');
    if exists(select 1 from portal_entitlements where user_id = p_user and revoked_at is null and expires_at <= now()) then s := 'EXPIRED'; end if;
  end if;
  return jsonb_build_object('state', s, 'pendingDays', n, 'activatedAt', e.activated_at, 'expiresAt', e.expires_at, 'isTest', false);
end $$;

-- 3. Atualizar portal_authorize (5 argumentos) para aceitar teste administrativo
create or replace function public.portal_authorize(
  p_user uuid,
  p_operation uuid,
  p_mode text,
  p_available boolean,
  p_verified_orders uuid[]
) returns jsonb
language plpgsql security definer set search_path=public,pg_temp as $$
declare
  tg portal_test_grants;
  e portal_entitlements;
  op portal_operations;
  s jsonb;
  permitted boolean := false;
begin
  if p_verified_orders is null then raise exception 'Verified order list is required' using errcode='22004'; end if;
  if p_mode not in ('inspect', 'execute') then raise exception 'invalid mode'; end if;
  perform pg_advisory_xact_lock(hashtextextended(p_user::text, 0));

  -- 1. Verificar se usuário possui concessão de teste ativa
  select * into tg from portal_test_grants where user_id = p_user and expires_at > now();
  if tg.user_id is not null then
    permitted := p_available;
    if p_mode = 'execute' and p_available then
      insert into portal_operations(user_id, operation_id, entitlement_id)
      values(p_user, p_operation, null)
      on conflict do nothing;
    end if;
    s := portal_subscription(p_user, p_available);
    return jsonb_build_object(
      'subject', p_user,
      'operation_id', p_operation,
      'allowed', permitted,
      'state', s->>'state',
      'expires_at', floor(extract(epoch from tg.expires_at)),
      'is_test', true
    );
  end if;

  -- 2. Fluxo comercial padrão para assinaturas pagas
  if p_mode = 'execute' then
    select * into op from portal_operations where user_id = p_user and operation_id = p_operation;
    if found then
      select * into e from portal_entitlements where id = op.entitlement_id and user_id = p_user
        and order_id = any(p_verified_orders);
    elsif p_available then
      select * into e from portal_entitlements where user_id = p_user and revoked_at is null and expires_at > now()
        and order_id = any(p_verified_orders) order by activated_at limit 1;
      if e.id is null then
        select * into e from portal_entitlements where user_id = p_user and revoked_at is null and activated_at is null
          and order_id = any(p_verified_orders) order by created_at, id limit 1 for update;
        if e.id is not null then
          update portal_entitlements set activated_at = now(), expires_at = now() + interval '2592000 seconds' where id = e.id returning * into e;
          update portal_orders set status = 'ACTIVE', updated_at = now() where id = e.order_id;
          insert into portal_events(user_id, order_id, event_key, kind) values(p_user, e.order_id, 'activation:' || e.id, 'ACTIVATED');
        end if;
      end if;
      insert into portal_operations(user_id, operation_id, entitlement_id) values(p_user, p_operation, e.id);
    end if;
  else
    select * into e from portal_entitlements where user_id = p_user and revoked_at is null and expires_at > now()
      and order_id = any(p_verified_orders) order by activated_at limit 1;
  end if;

  permitted := p_available and e.id is not null and e.revoked_at is null and e.expires_at > now();
  s := portal_subscription(p_user, p_available);
  return jsonb_build_object(
    'subject', p_user,
    'operation_id', p_operation,
    'allowed', permitted,
    'state', s->>'state',
    'expires_at', case when e.id is not null then floor(extract(epoch from e.expires_at)) else null end,
    'is_test', false
  );
end $$;

revoke all on function public.portal_authorize(uuid,uuid,text,boolean,uuid[]) from public,anon,authenticated;
grant execute on function public.portal_authorize(uuid,uuid,text,boolean,uuid[]) to service_role;
notify pgrst, 'reload schema';
commit;
