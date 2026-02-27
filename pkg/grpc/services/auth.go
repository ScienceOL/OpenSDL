package services

import (
	osdlv1 "github.com/scienceol/osdl/gen/osdl/v1"
)

type AuthService struct {
	osdlv1.UnimplementedAuthServiceServer
}

func NewAuthService() *AuthService {
	return &AuthService{}
}
