"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import { getAccessToken } from "@/lib/token-store";

interface UseWebSocketOptions {
  /** WebSocket URL (e.g. "ws://localhost:8080/ws") */
  url: string;
  /** Auto-reconnect on disconnect (default: true) */
  autoReconnect?: boolean;
  /** Max reconnect delay in ms (default: 30000) */
  maxReconnectDelay?: number;
  /** Max reconnect attempts before giving up (default: 20) */
  maxReconnectAttempts?: number;
  /** Send auth token as first message after connect instead of in URL (default: true) */
  withAuth?: boolean;
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

    // Connect without token in URL - send auth as first message instead
    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onopen = () => {
      setReadyState(WebSocket.OPEN);
      reconnectAttempt.current = 0;

      // Send auth token as first message (not in URL to avoid log exposure)
      if (withAuth) {
        const token = getAccessToken();
        if (token) {
          ws.send(JSON.stringify({ type: "auth", token }));
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
  }, [url, withAuth, autoReconnect, maxReconnectDelay, maxReconnectAttempts]);

  useEffect(() => {
    connect();
    return () => {
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      if (wsRef.current) {
        wsRef.current.onclose = null; // prevent reconnect on intentional close
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
