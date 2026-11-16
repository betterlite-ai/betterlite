package litellm

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestNewClientSetsDefaultRequestTimeout(t *testing.T) {
	client := NewClient("http://localhost:4000", "sk-test", false)

	if client.httpClient.Timeout != 300*time.Second {
		t.Fatalf("http.Client.Timeout = %s, want 5m0s", client.httpClient.Timeout)
	}
}

func TestRequestFailsWhenServerNeverAnswers(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		<-r.Context().Done()
	}))
	defer server.Close()

	client := newClient(server.URL, "sk-test", false, clientLimits{
		requestTimeout:      200 * time.Millisecond,
		dialTimeout:         time.Second,
		tlsHandshakeTimeout: time.Second,
		maxResponseBytes:    1 << 20,
	})

	start := time.Now()
	_, err := client.GetKey("sk-test")
	elapsed := time.Since(start)

	if err == nil {
		t.Fatal("GetKey succeeded against a server that never answers")
	}
	if elapsed < 100*time.Millisecond {
		t.Errorf("GetKey failed after %s, want at least 100ms", elapsed)
	}
	if elapsed > 5*time.Second {
		t.Errorf("GetKey failed after %s, want before 5s", elapsed)
	}
}

func TestRequestFailsWhenResponseBodyStalls(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		w.(http.Flusher).Flush()
		<-r.Context().Done()
	}))
	defer server.Close()

	client := newClient(server.URL, "sk-test", false, clientLimits{
		requestTimeout:      200 * time.Millisecond,
		dialTimeout:         time.Second,
		tlsHandshakeTimeout: time.Second,
		maxResponseBytes:    1 << 20,
	})

	start := time.Now()
	resp, err := MakeRequest(client, "GET", "/stall", nil)
	if err != nil {
		t.Fatalf("MakeRequest /stall: %v", err)
	}
	defer resp.Body.Close()

	_, err = io.ReadAll(resp.Body)
	elapsed := time.Since(start)
	if err == nil {
		t.Fatal("reading a stalled response body succeeded")
	}

	if elapsed < 100*time.Millisecond {
		t.Errorf("body read failed after %s, want at least 100ms", elapsed)
	}
	if elapsed > 5*time.Second {
		t.Errorf("body read failed after %s, want before 5s", elapsed)
	}
}

func TestResponseBodyOverCapFails(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/at-cap" {
			_, _ = w.Write(make([]byte, 1024))
			return
		}
		_, _ = w.Write(make([]byte, 1025))
	}))
	defer server.Close()

	client := newClient(server.URL, "sk-test", false, clientLimits{
		requestTimeout:      5 * time.Second,
		dialTimeout:         time.Second,
		tlsHandshakeTimeout: time.Second,
		maxResponseBytes:    1024,
	})

	atCap, err := MakeRequest(client, "GET", "/at-cap", nil)
	if err != nil {
		t.Fatalf("MakeRequest /at-cap: %v", err)
	}
	body, err := io.ReadAll(atCap.Body)
	_ = atCap.Body.Close()
	if err != nil {
		t.Fatalf("reading at-cap body: %v", err)
	}
	if len(body) != 1024 {
		t.Errorf("at-cap body length = %d, want 1024", len(body))
	}

	overCap, err := MakeRequest(client, "GET", "/over-cap", nil)
	if err != nil {
		t.Fatalf("MakeRequest /over-cap: %v", err)
	}
	_, err = io.ReadAll(overCap.Body)
	_ = overCap.Body.Close()
	if err == nil {
		t.Fatal("reading an over-cap body succeeded")
	}
	if !strings.Contains(err.Error(), "1024") {
		t.Errorf("over-cap read error = %q, want it to name 1024", err)
	}

	if _, err := client.GetKey("sk-test"); err == nil {
		t.Fatal("GetKey succeeded against an over-cap response")
	} else if !strings.Contains(err.Error(), "1024") {
		t.Errorf("GetKey error = %q, want it to name 1024", err)
	}
}
