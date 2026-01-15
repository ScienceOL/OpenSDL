package repo

import "context"

// CompoundInfo holds the basic information for a chemical compound.
type CompoundInfo struct {
	Name             string `json:"name"`
	MolecularFormula string `json:"molecular_formula"`
	SMILES           string `json:"smiles"`
}

// PubChemRepo defines the interface for interacting with the PubChem API.
type PubChemRepo interface {
	GetCompoundByCAS(ctx context.Context, cas string) (*CompoundInfo, error)
}
