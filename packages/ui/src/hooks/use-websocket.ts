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
  /** Include auth token as query param (default: true) */
  withAuth?: boolean;
  /** Called when a message is received */
  onMessage?: (data: unknown) => void;
  /** Called on connection open */
  onOpen?: () => void;
  /** Called on connection close */
  onClose?: () => void;
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
  withAuth = true,
  onMessage,
  onOpen,
  onClose,
}: UseWebSocketOptions): UseWebSocketReturn {
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectAttempt = useRef(0);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout>>();
  const [lastMessage, setLastMessage] = useState<unknown | null>(null);
  const [readyState, setReadyState] = useState(WebSocket.CLOSED);

  const connect = useCallback(() => {
    if (typeof window === "undefined") return;

    let wsUrl = url;
    if (withAuth) {
      const token = getAccessToken();
      if (token) {
        const sep = wsUrl.includes("?") ? "&" : "?";
        wsUrl = `${wsUrl}${sep}token=${token}`;
      }
    }

    const ws = new WebSocket(wsUrl);
    wsRef.current = ws;

    ws.onopen = () => {
      setReadyState(WebSocket.OPEN);
      reconnectAttempt.current = 0;
      onOpen?.();
    };

    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        setLastMessage(data);
        onMessage?.(data);
      } catch {
        setLastMessage(event.data);
        onMessage?.(event.data);
      }
    };

    ws.onclose = () => {
      setReadyState(WebSocket.CLOSED);
      onClose?.();

      if (autoReconnect) {
        const delay = Math.min(1000 * 2 ** reconnectAttempt.current, maxReconnectDelay);
        reconnectAttempt.current++;
        reconnectTimer.current = setTimeout(connect, delay);
      }
    };

    ws.onerror = () => {
      ws.close();
    };
  }, [url, withAuth, autoReconnect, maxReconnectDelay, onMessage, onOpen, onClose]);

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
