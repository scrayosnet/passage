---
title: Kubernetes Deployment
description: Deploy Passage on Kubernetes with high availability and auto-scaling.
---

This guide covers deploying Passage on Kubernetes, from basic setups to production-grade configurations with Agones integration.

## Prerequisites

- Kubernetes cluster (1.25+)
- `kubectl` configured
- Basic understanding of Kubernetes concepts
- (Optional) Helm 3.x for chart-based deployment
- (Optional) Agones installed for game server discovery

## Quick Start

### Basic Deployment

Create a minimal Passage deployment:

```yaml
# passage-deployment.yaml
apiVersion: v1
kind: Namespace
metadata:
  name: passage
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: passage-config
  namespace: passage
data:
  config.toml: |
    address = "0.0.0.0:25565"
    timeout = 120

    [status]
    adapter = "fixed"
    [status.fixed]
    name = "My Kubernetes Network"
    description = "\"Powered by Passage\""

    [target_discovery]
    adapter = "fixed"
    [[target_discovery.fixed.targets]]
    identifier = "hub-1"
    address = "hub-service.minecraft:25565"

    [target_strategy]
    adapter = "fixed"

    [localization]
    default_locale = "en_US"
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: passage
  namespace: passage
spec:
  replicas: 2
  selector:
    matchLabels:
      app: passage
  template:
    metadata:
      labels:
        app: passage
    spec:
      containers:
      - name: passage
        image: ghcr.io/scrayosnet/passage:latest
        ports:
        - containerPort: 25565
          protocol: TCP
        resources:
          requests:
            memory: "64Mi"
            cpu: "100m"
          limits:
            memory: "256Mi"
            cpu: "500m"
        volumeMounts:
        - name: config
          mountPath: /app/config
          readOnly: true
        env:
        - name: RUST_LOG
          value: "info"
      volumes:
      - name: config
        configMap:
          name: passage-config
---
apiVersion: v1
kind: Service
metadata:
  name: passage
  namespace: passage
spec:
  type: LoadBalancer
  ports:
  - port: 25565
    targetPort: 25565
    protocol: TCP
    name: minecraft
  selector:
    app: passage
```

Deploy:
```bash
kubectl apply -f passage-deployment.yaml
```

Get the external IP:
```bash
kubectl get svc passage -n passage
```

## Production Configuration

### High Availability Setup

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: passage
  namespace: passage
spec:
  replicas: 3  # Multiple instances
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 0  # Zero-downtime updates
  selector:
    matchLabels:
      app: passage
  template:
    metadata:
      labels:
        app: passage
    spec:
      # Anti-affinity: spread across nodes
      affinity:
        podAntiAffinity:
          preferredDuringSchedulingIgnoredDuringExecution:
          - weight: 100
            podAffinityTerm:
              labelSelector:
                matchExpressions:
                - key: app
                  operator: In
                  values:
                  - passage
              topologyKey: kubernetes.io/hostname
      containers:
      - name: passage
        image: ghcr.io/scrayosnet/passage:v0.1.24
        ports:
        - containerPort: 25565
          protocol: TCP
        resources:
          requests:
            memory: "128Mi"
            cpu: "200m"
          limits:
            memory: "512Mi"
            cpu: "1000m"
        # Health checks
        livenessProbe:
          tcpSocket:
            port: 25565
          initialDelaySeconds: 10
          periodSeconds: 30
        readinessProbe:
          tcpSocket:
            port: 25565
          initialDelaySeconds: 5
          periodSeconds: 10
        volumeMounts:
        - name: config
          mountPath: /app/config
          readOnly: true
        - name: auth-secret
          mountPath: /run/secrets
          readOnly: true
        env:
        - name: RUST_LOG
          value: "info"
        - name: AUTH_SECRET_FILE
          value: "/run/secrets/auth-secret"
      volumes:
      - name: config
        configMap:
          name: passage-config
      - name: auth-secret
        secret:
          secretName: passage-auth-secret
```

### Authentication Secret

```bash
# Generate auth secret
openssl rand -base64 32 > auth_secret

# Create Kubernetes secret
kubectl create secret generic passage-auth-secret \
  --from-file=auth-secret=auth_secret \
  -n passage

# Clean up local file
rm auth_secret
```

### Resource Limits

Recommended resource allocation:

| Scenario | CPU Request | CPU Limit | Memory Request | Memory Limit |
|----------|-------------|-----------|----------------|--------------|
| Small (<100 players/min) | 100m | 500m | 64Mi | 256Mi |
| Medium (<500 players/min) | 200m | 1000m | 128Mi | 512Mi |
| Large (<2000 players/min) | 500m | 2000m | 256Mi | 1Gi |

## Agones Integration

### Installing Agones

```bash
# Add Agones Helm repository
helm repo add agones https://agones.dev/chart/stable
helm repo update

# Install Agones
kubectl create namespace agones-system
helm install agones --namespace agones-system agones/agones
```

### RBAC Permissions

Passage needs permissions to watch GameServers:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: passage
  namespace: passage
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: passage-agones
  namespace: minecraft  # Namespace where GameServers are
rules:
- apiGroups: ["agones.dev"]
  resources: ["gameservers"]
  verbs: ["get", "list", "watch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: passage-agones
  namespace: minecraft
subjects:
- kind: ServiceAccount
  name: passage
  namespace: passage
roleRef:
  kind: Role
  name: passage-agones
  apiGroup: rbac.authorization.k8s.io
```

### Passage Configuration for Agones

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: passage-config
  namespace: passage
data:
  config.toml: |
    address = "0.0.0.0:25565"
    timeout = 120

    [status]
    adapter = "http"
    [status.http]
    address = "http://status-service.minecraft/status"
    cache_duration = 5

    [target_discovery]
    adapter = "agones"
    [target_discovery.agones]
    namespace = "minecraft"
    label_selector = "game=minecraft,type=lobby"

    [target_strategy]
    adapter = "player_fill"
    [target_strategy.player_fill]
    field = "players"
    max_players = 50

    [rate_limiter]
    enabled = true
    duration = 60
    size = 100

    [localization]
    default_locale = "en_US"
```

### Example GameServer Fleet

```yaml
apiVersion: "agones.dev/v1"
kind: Fleet
metadata:
  name: minecraft-lobbies
  namespace: minecraft
spec:
  replicas: 5
  template:
    metadata:
      labels:
        game: minecraft
        type: lobby
    spec:
      ports:
      - name: minecraft
        containerPort: 25565
        protocol: TCP
      counters:
        players:
          count: 0
          capacity: 50
      template:
        spec:
          containers:
          - name: minecraft
            image: itzg/minecraft-server:latest
            env:
            - name: EULA
              value: "TRUE"
            - name: TYPE
              value: "PAPER"
```

## Networking

### LoadBalancer Service

```yaml
apiVersion: v1
kind: Service
metadata:
  name: passage
  namespace: passage
spec:
  type: LoadBalancer
  externalTrafficPolicy: Local  # Preserve client IP
  ports:
  - port: 25565
    targetPort: 25565
    protocol: TCP
    name: minecraft
  selector:
    app: passage
```

### NodePort Service

For on-premise or specific cloud setups:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: passage
  namespace: passage
spec:
  type: NodePort
  ports:
  - port: 25565
    targetPort: 25565
    nodePort: 30565  # Must be in 30000-32767 range
    protocol: TCP
  selector:
    app: passage
```

### Ingress (Not Recommended)

Minecraft protocol is TCP-based and doesn't work well with HTTP-based ingress controllers. Use LoadBalancer or NodePort instead.

## PROXY Protocol

If using a load balancer that supports PROXY protocol:

```yaml
# In passage-config ConfigMap
[proxy_protocol]
enabled = true
```

AWS NLB example:
```yaml
apiVersion: v1
kind: Service
metadata:
  name: passage
  namespace: passage
  annotations:
    service.beta.kubernetes.io/aws-load-balancer-type: "nlb"
    service.beta.kubernetes.io/aws-load-balancer-proxy-protocol: "*"
spec:
  type: LoadBalancer
  externalTrafficPolicy: Local
  ports:
  - port: 25565
    targetPort: 25565
    protocol: TCP
  selector:
    app: passage
```

## Horizontal Pod Autoscaling

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: passage
  namespace: passage
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: passage
  minReplicas: 2
  maxReplicas: 10
  metrics:
  - type: Resource
    resource:
      name: cpu
      target:
        type: Utilization
        averageUtilization: 70
  - type: Resource
    resource:
      name: memory
      target:
        type: Utilization
        averageUtilization: 80
```

## Monitoring

### Prometheus ServiceMonitor

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: passage
  namespace: passage
spec:
  selector:
    matchLabels:
      app: passage
  endpoints:
  - port: metrics
    interval: 30s
```

### OpenTelemetry Configuration

```yaml
# In passage-config ConfigMap
[otel]
environment = "production"
traces_endpoint = "http://tempo.monitoring:4318/v1/traces"
traces_token = ""
metrics_endpoint = "http://mimir.monitoring:4318/v1/metrics"
metrics_token = ""
```

## Troubleshooting

### Pods Not Starting

```bash
# Check pod status
kubectl get pods -n passage

# View logs
kubectl logs -n passage -l app=passage

# Describe pod for events
kubectl describe pod -n passage <pod-name>
```

### Can't Connect from Minecraft Client

```bash
# Check service
kubectl get svc passage -n passage

# Check if LoadBalancer got external IP
kubectl get svc passage -n passage -o jsonpath='{.status.loadBalancer.ingress[0]}'

# Test connection to pod directly
kubectl port-forward -n passage svc/passage 25565:25565
```

### Agones Discovery Not Working

```bash
# Check RBAC permissions
kubectl auth can-i watch gameservers \
  --as=system:serviceaccount:passage:passage \
  -n minecraft

# Check GameServers exist
kubectl get gameservers -n minecraft

# Check Passage logs for Agones errors
kubectl logs -n passage -l app=passage | grep agones
```

### High CPU Usage

- Check rate limiter is enabled
- Review adapter response times
- Consider increasing replicas instead of resources

### Memory Leak

Passage is stateless and should have constant memory. If memory grows:
- Check for goroutine leaks (shouldn't happen in Rust)
- Review logs for errors
- File a bug report with metrics

## Best Practices

### Security

✅ Use NetworkPolicies to restrict traffic
✅ Run with non-root user (Passage default)
✅ Store secrets in Kubernetes Secrets
✅ Use RBAC with minimal permissions
✅ Enable Pod Security Standards

### Reliability

✅ Run multiple replicas (minimum 2)
✅ Use pod anti-affinity
✅ Configure health checks
✅ Set resource requests and limits
✅ Enable HPA for auto-scaling

### Performance

✅ Use `externalTrafficPolicy: Local` for LoadBalancer
✅ Enable rate limiting
✅ Right-size resource requests
✅ Monitor with Prometheus/OpenTelemetry

### Operations

✅ Use specific image tags (not `latest`)
✅ Implement zero-downtime rolling updates
✅ Configure logging aggregation
✅ Set up alerting for key metrics
✅ Document your configuration
