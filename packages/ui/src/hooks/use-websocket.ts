"use client";

import { useEffect, useRef, useState, useCallback } from "react";

interface UseWebSocketOptions {
  /** WebSocket URL (e.g. "wss://localhost:8080/ws") */
  url: string;
  /** Auto-reconnect on disconnect (default: true) */
  autoReconnect?: boolean;
  /** Max reconnect delay in ms (default: 30000) */
  maxReconnectDelay?: number;
  /** Max reconnect attempts before giving up (default: 20) */
  maxReconnectAttempts?: number;
  /** Authenticate via a server-issued ticket (default: true).
   *  Calls ticketEndpoint to get a single-use ticket, then sends it
   *  as the first WebSocket message. The server validates and maps
   *  the ticket to a session. */
  withAuth?: boolean;
  /** Endpoint that returns { ticket: string } (default: "/api/auth/ws-ticket") */
  ticketEndpoint?: string;
  /** Called when a message is received */
  onMessage?: (data: unknown) => void;
  /** Called on connection open */
  onOpen?: () => void;
  /** Called on connection close */
  onClose?: () => void;
  /** Called when max reconnect attempts exhausted */
  onReconnectFailed?: () => void;
}

interface UseWebSocketReturn {
  /** Send a JSON message */
  send: (data: unknown) => void;
  /** Last received message (parsed JSON) */
  lastMessage: unknown | null;
  /** Connection state */
  readyState: number;
  /** Whether connected */
  isConnected: boolean;
}

export function useWebSocket({
  url,
  autoReconnect = true,
  maxReconnectDelay = 30_000,
  maxReconnectAttempts = 20,
  withAuth = true,
  ticketEndpoint = "/api/auth/ws-ticket",
  onMessage,
  onOpen,
  onClose,
  onReconnectFailed,
}: UseWebSocketOptions): UseWebSocketReturn {
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectAttempt = useRef(0);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout>>();
  const [lastMessage, setLastMessage] = useState<unknown | null>(null);
  const [readyState, setReadyState] = useState(WebSocket.CLOSED);

  // Store callbacks in refs to avoid re-creating the connection on every render
  const onMessageRef = useRef(onMessage);
  const onOpenRef = useRef(onOpen);
  const onCloseRef = useRef(onClose);
  const onReconnectFailedRef = useRef(onReconnectFailed);
  useEffect(() => { onMessageRef.current = onMessage; }, [onMessage]);
  useEffect(() => { onOpenRef.current = onOpen; }, [onOpen]);
  useEffect(() => { onCloseRef.current = onClose; }, [onClose]);
  useEffect(() => { onReconnectFailedRef.current = onReconnectFailed; }, [onReconnectFailed]);

  const connect = useCallback(() => {
    if (typeof window === "undefined") return;

    // Enforce WSS in production
    if (process.env.NODE_ENV === "production" && url.startsWith("ws://")) {
      console.error("Refusing insecure WebSocket connection in production. Use wss://");
      return;
    }

    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onopen = async () => {
      setReadyState(WebSocket.OPEN);
      reconnectAttempt.current = 0;

      // Authenticate via single-use ticket (not raw token)
      if (withAuth) {
        try {
          const res = await fetch(ticketEndpoint, {
            method: "POST",
            credentials: "include",
            headers: { "Content-Type": "application/json" },
          });
          if (res.ok) {
            const { ticket } = await res.json();
            ws.send(JSON.stringify({ type: "auth", ticket }));
          }
        } catch {
          // Auth failed - connection will proceed unauthenticated
        }
      }

      onOpenRef.current?.();
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        setLastMessage(data);
        onMessageRef.current?.(data);
      } catch {
        setLastMessage(event.data);
        onMessageRef.current?.(event.data);
      }
    };

    ws.onclose = () => {
      setReadyState(WebSocket.CLOSED);
      onCloseRef.current?.();

      if (autoReconnect && reconnectAttempt.current < maxReconnectAttempts) {
        const delay = Math.min(1000 * 2 ** reconnectAttempt.current, maxReconnectDelay);
        reconnectAttempt.current++;
        reconnectTimer.current = setTimeout(connect, delay);
      } else if (reconnectAttempt.current >= maxReconnectAttempts) {
        onReconnectFailedRef.current?.();
      }
    };

    ws.onerror = () => {
      ws.close();
    };
  }, [url, withAuth, ticketEndpoint, autoReconnect, maxReconnectDelay, maxReconnectAttempts]);

  useEffect(() => {
    connect();
    return () => {
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      if (wsRef.current) {
        wsRef.current.onclose = null;
        wsRef.current.close();
      }
    };
  }, [connect]);

  const send = useCallback((data: unknown) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(typeof data === "string" ? data : JSON.stringify(data));
    }
  }, []);

  return {
    send,
    lastMessage,
    readyState,
    isConnected: readyState === WebSocket.OPEN,
  };
}
