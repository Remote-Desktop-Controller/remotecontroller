"use client";

import {createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode} from "react";
import type {SessionUser} from "../lib/auth-session";

type AuthStatus = "loading" | "authenticated" | "guest" | "error";
type AuthState = {status:AuthStatus;user:SessionUser|null;error:string};
type AuthContextValue = AuthState & {isLoggingOut:boolean;refresh:()=>Promise<SessionUser|null>;logout:()=>Promise<void>};
const initialState:AuthState={status:"loading",user:null,error:""};
const SessionContext=createContext<AuthContextValue|null>(null);
const sessionError="Não foi possível verificar sua sessão. Tente novamente.";

export function SessionProvider({children}:{children:ReactNode}){
  const[state,setState]=useState<AuthState>(initialState);
  const[isLoggingOut,setIsLoggingOut]=useState(false);
  const generation=useRef(0);
  const loggingOut=useRef(false);
  const pendingLogout=useRef<Promise<void>|null>(null);
  const controller=useRef<AbortController|null>(null);
  const channel=useRef<BroadcastChannel|null>(null);
  const invalidate=useCallback(()=>{++generation.current;controller.current?.abort();},[]);
  const refresh=useCallback(async()=>{
    if(loggingOut.current)return null;
    const request=++generation.current;
    controller.current?.abort();
    const abort=new AbortController();
    controller.current=abort;
    try{
      const response=await fetch("/api/auth/session",{credentials:"same-origin",cache:"no-store",signal:abort.signal});
      if(!response.ok)throw new Error(sessionError);
      const data=await response.json() as {user:SessionUser|null};
      if(data.user!==null&&(!data.user||!data.user.id||!data.user.email||data.user.emailVerified!==true||!data.user.confirmedAt))throw new Error(sessionError);
      if(request!==generation.current||loggingOut.current)return null;
      setState({status:data.user?"authenticated":"guest",user:data.user,error:""});
      setIsLoggingOut(false);
      return data.user;
    }catch(error){
      if(request!==generation.current||loggingOut.current)return null;
      setState({status:"error",user:null,error:sessionError});
      throw error;
    }
  },[]);
  const logout=useCallback(()=>{
    if(pendingLogout.current)return pendingLogout.current;
    loggingOut.current=true;
    setIsLoggingOut(true);
    invalidate();
    setState(initialState);
    const operation=(async()=>{
    try{
      const response=await fetch("/api/auth/logout",{method:"POST",credentials:"same-origin",cache:"no-store",headers:{"Content-Type":"application/json"},body:"{}"});
      if(!response.ok){const data=await response.json().catch(()=>({}));throw new Error(data.message||"Não foi possível sair da conta. Tente novamente.");}
      setState({status:"guest",user:null,error:""});
      channel.current?.postMessage("invalidate");
    }catch(error){
      setIsLoggingOut(false);
      setState({status:"error",user:null,error:(error as Error).message});
      throw error;
    }finally{loggingOut.current=false;pendingLogout.current=null;}
    })();
    pendingLogout.current=operation;
    return operation;
  },[invalidate]);
  useEffect(()=>{
    const reload=()=>{void refresh().catch(()=>{});};
    const visible=()=>{if(document.visibilityState==="visible")reload();};
    reload();
    window.addEventListener("focus",reload);
    window.addEventListener("pageshow",reload);
    document.addEventListener("visibilitychange",visible);
    if(typeof BroadcastChannel!=="undefined"){
      channel.current=new BroadcastChannel("remotecontroller-session");
      channel.current.onmessage=event=>{if(event.data==="invalidate")reload();};
    }
    return()=>{
      invalidate();
      window.removeEventListener("focus",reload);
      window.removeEventListener("pageshow",reload);
      document.removeEventListener("visibilitychange",visible);
      channel.current?.close();
      channel.current=null;
    };
  },[refresh,invalidate]);
  return <SessionContext.Provider value={{...state,isLoggingOut,refresh,logout}}>{children}</SessionContext.Provider>;
}

export function useAuth(){
  const context=useContext(SessionContext);
  if(!context)throw new Error("SessionProvider é necessário para consultar a sessão.");
  return context;
}
