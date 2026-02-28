package grpc

import (
	"context"
	"strconv"
	"strings"
	"time"

	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/middleware/auth"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/repo/model"
	"github.com/scienceol/osdl/pkg/utils"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
)

type userContextKey struct{}

// skipAuth returns true for services that should not require authentication.
func skipAuth(fullMethod string) bool {
	return strings.HasPrefix(fullMethod, "/grpc.reflection.") ||
		strings.HasPrefix(fullMethod, "/grpc.health.")
}

func UserFromContext(ctx context.Context) *model.UserData {
	u, _ := ctx.Value(userContextKey{}).(*model.UserData)
	return u
}

func extractAndValidateToken(ctx context.Context) (*model.UserData, error) {
	md, ok := metadata.FromIncomingContext(ctx)
	if !ok {
		return nil, status.Error(codes.Unauthenticated, "missing metadata")
	}

	values := md.Get("authorization")
	if len(values) == 0 {
		return nil, status.Error(codes.Unauthenticated, "missing authorization header")
	}

	authHeader := values[0]
	parts := strings.SplitN(authHeader, " ", 2)
	if len(parts) != 2 {
		return nil, status.Error(codes.Unauthenticated, "invalid authorization format")
	}

	tokenType := parts[0]
	token := parts[1]

	switch auth.AuthType(tokenType) {
	case auth.AuthTypeBearer:
		if config.Global().Auth.AuthSource == config.AuthBohr {
			claims := &utils.Claims{}
			if err := utils.ParseJWTWithPublicKey(token, utils.DefaultPublicKey, claims); err != nil {
				logger.Errorf(ctx, "gRPC auth: bohr JWT validation failed: %v", err)
				return nil, status.Error(codes.Unauthenticated, "invalid token")
			}
			if claims.Exp <= time.Now().UTC().Unix() {
				return nil, status.Error(codes.Unauthenticated, "token expired")
			}
			return &model.UserData{
				ID:    strconv.FormatUint(claims.Identity.UserID, 10),
				OrgID: strconv.FormatUint(claims.Identity.OrgID, 10),
			}, nil
		}
		user, err := auth.ValidateToken(ctx, tokenType, token)
		if err != nil {
			logger.Errorf(ctx, "gRPC auth: bearer token validation failed: %v", err)
			return nil, status.Error(codes.Unauthenticated, "invalid token")
		}
		return user, nil
	default:
		return nil, status.Errorf(codes.Unauthenticated, "unsupported auth type: %s", tokenType)
	}
}

func UnaryAuthInterceptor() grpc.UnaryServerInterceptor {
	return func(ctx context.Context, req any, info *grpc.UnaryServerInfo, handler grpc.UnaryHandler) (any, error) {
		if skipAuth(info.FullMethod) {
			return handler(ctx, req)
		}
		user, err := extractAndValidateToken(ctx)
		if err != nil {
			return nil, err
		}
		newCtx := context.WithValue(ctx, userContextKey{}, user)
		return handler(newCtx, req)
	}
}

func StreamAuthInterceptor() grpc.StreamServerInterceptor {
	return func(srv any, ss grpc.ServerStream, info *grpc.StreamServerInfo, handler grpc.StreamHandler) error {
		if skipAuth(info.FullMethod) {
			return handler(srv, ss)
		}
		user, err := extractAndValidateToken(ss.Context())
		if err != nil {
			return err
		}
		newCtx := context.WithValue(ss.Context(), userContextKey{}, user)
		wrapped := &wrappedStream{ServerStream: ss, ctx: newCtx}
		return handler(srv, wrapped)
	}
}

type wrappedStream struct {
	grpc.ServerStream
	ctx context.Context
}

func (w *wrappedStream) Context() context.Context {
	return w.ctx
}
